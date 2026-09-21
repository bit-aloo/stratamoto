use std::{cell::RefCell, fs::OpenOptions, io::Write, rc::Rc};

use clap::Parser;
use libafl::{
    Error,
    events::{ClientDescription, SimpleEventManager},
    monitors::{Monitor, tui::TuiMonitor},
};

#[cfg(not(feature = "simplemgr"))]
use libafl::events::{EventConfig, Launcher};

use libafl_bolts::{core_affinity::CoreId, current_time};

#[cfg(not(feature = "simplemgr"))]
use libafl_bolts::shmem::{ShMemProvider, StdShMemProvider};

use crate::{client::Client, monitor::GlobalMonitor, options::FuzzerOptions};

pub struct Fuzzer {
    options: FuzzerOptions,
}

impl Fuzzer {
    pub fn new() -> Fuzzer {
        let options = FuzzerOptions::parse();
        Fuzzer { options }
    }

    pub fn fuzz(&self) -> Result<(), Error> {
        if self.options.tui {
            let monitor = TuiMonitor::builder()
                .title("stratamoto IR fuzzer")
                .version(env!("CARGO_PKG_VERSION"))
                .enhanced_graphics(true)
                .build();
            self.launch(monitor)
        } else {
            let log = self.options.log.as_ref().and_then(|l| {
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(l)
                    .ok()
                    .map(RefCell::new)
                    .map(Rc::new)
            });

            let log_fn = {
                let log = log.clone();
                move |s: &str| {
                    println!("{s}");

                    if let Some(log) = &log {
                        writeln!(log.borrow_mut(), "{:?} {}", current_time(), s).unwrap();
                    }
                }
            };

            self.launch(GlobalMonitor::new(log_fn))
        }
    }

    fn launch<M>(&self, monitor: M) -> Result<(), Error>
    where
        M: Monitor + Clone,
    {
        #[cfg(not(feature = "simplemgr"))]
        let shmem_provider = StdShMemProvider::new()?;

        // If we are running verbose, don't provide a replacement stdout, otherwise use
        // /dev/null.
        #[cfg(not(feature = "simplemgr"))]
        let stdout = if self.options.verbose {
            None
        } else {
            Some("/dev/null")
        };

        let client = Client::new(&self.options);

        #[cfg(not(feature = "simplemgr"))]
        if self.options.rerun_input.is_some() || self.options.minimize_input.is_some() {
            // To rerun an input, instead of using a launcher, we create dummy parameters and
            // run the client directly.
            return client.run(
                None,
                SimpleEventManager::new(monitor.clone()),
                ClientDescription::new(0, 0, CoreId(0)),
            );
        }

        #[cfg(feature = "simplemgr")]
        return client.run(
            None,
            SimpleEventManager::new(monitor),
            ClientDescription::new(0, 0, CoreId(0)),
        );

        #[cfg(not(feature = "simplemgr"))]
        match Launcher::builder()
            .shmem_provider(shmem_provider)
            .broker_port(self.options.port)
            .configuration(EventConfig::from_build_id())
            // With an existing corpus and many clients, crashes in the llmp broker have been
            // observed; a launch delay or fewer clients mitigate it.
            .launch_delay(self.options.launch_delay)
            .monitor(monitor)
            .run_client(|s, m, c| client.run(s, m, c))
            .cores(&self.options.cores)
            .stdout_file(stdout)
            .stderr_file(stdout)
            .build()
            .launch()
        {
            Ok(()) => Ok(()),
            Err(Error::ShuttingDown) => {
                println!("Fuzzing stopped by user. Good bye.");
                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}
