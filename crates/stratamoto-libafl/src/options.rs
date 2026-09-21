use std::path::PathBuf;

use clap::Parser;
use libafl_bolts::core_affinity::{CoreId, Cores};
use rand::RngExt;

#[derive(Parser, Debug)]
#[command(name = "stratamoto-libafl", about = "stratamoto IR fuzzer", long_about = None)]
pub struct FuzzerOptions {
    #[arg(short, long, help = "Input directory")]
    pub input: String,

    #[arg(short, long, help = "Output directory")]
    pub output: String,

    #[arg(short, long, help = "Nyx share directory")]
    pub share: String,

    #[arg(short, long, help = "Input buffer size", default_value_t = 8_388_608)]
    pub buffer_size: usize,

    #[arg(long, help = "Log file")]
    pub log: Option<String>,

    #[arg(long, help = "Timeout in milliseconds", default_value = "1000")]
    pub timeout: u32,

    #[arg(long, help = "Don't report hangs as bugs", default_value_t = false)]
    pub ignore_hangs: bool,

    #[arg(
        long,
        help = "Multiplier applied to the timeout to confirm a hang (hang_timeout = hang_multiple * timeout)",
        default_value_t = 5
    )]
    pub hang_multiple: u32,

    #[arg(
        long,
        help = "Client launch delay in milliseconds",
        default_value = "1000"
    )]
    pub launch_delay: u64,

    #[arg(long = "port", help = "Broker port", default_value_t = 1337_u16)]
    pub port: u16,

    #[arg(long, help = "CPU cores to use", default_value = "all", value_parser = Cores::from_cmdline)]
    pub cores: Cores,

    #[arg(
        long,
        help = "Don't add new inputs to the corpus",
        default_value_t = false
    )]
    pub static_corpus: bool,

    #[arg(
        long,
        help = "Remove disabled corpus entries after the initial load",
        default_value_t = false
    )]
    pub prune_disabled: bool,

    #[arg(
        long,
        help = "Number of corpus entries cached in memory",
        env = "STRATAMOTO_CORPUS_CACHE",
        default_value_t = 100
    )]
    pub corpus_cache: usize,

    #[arg(short, long, help = "Enable output from the fuzzer clients")]
    pub verbose: bool,

    #[arg(long, help = "Enable AFL++ style output", conflicts_with = "verbose")]
    pub tui: bool,

    #[arg(long = "iterations", help = "Maximum number of iterations")]
    pub iterations: Option<u64>,

    #[arg(
        short = 'r',
        help = "An input to rerun, instead of starting to fuzz. Ignores all other settings."
    )]
    pub rerun_input: Option<PathBuf>,

    #[arg(short = 'm', long, help = "An input to minimize")]
    pub minimize_input: Option<PathBuf>,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated list of mutators/generators to enable (all by default)"
    )]
    pub mutators: Option<Vec<String>>,

    #[arg(
        long,
        help = "Probability of enabling a generator/mutator in swarm testing mode",
        default_value_t = 1.0,
        value_parser = |v: &str| {
            let p: f64 = v.parse().map_err(|_| "swarm must be a number between 0.0 and 1.0")?;
            if (0.0..=1.0).contains(&p) {
                Ok(p)
            } else {
                Err("swarm must be a number between 0.0 and 1.0")
            }
        }
    )]
    pub swarm: f64,

    #[arg(
        long,
        help = "Seed for swarm testing (defaults to the current Unix time)",
        default_value_t = unix_time()
    )]
    pub swarm_seed: u64,

    #[arg(
        long,
        help = "Number of roles programs address, when the scenario has not dumped its context",
        default_value_t = 1
    )]
    pub roles: usize,
}

fn unix_time() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

impl FuzzerOptions {
    pub fn input_dir(&self) -> PathBuf {
        PathBuf::from(&self.input)
    }

    pub fn shared_dir(&self) -> PathBuf {
        PathBuf::from(&self.share)
    }

    pub fn output_dir(&self, core_id: CoreId) -> PathBuf {
        let mut dir = PathBuf::from(&self.output);
        dir.push(format!("cpu_{:03}", core_id.0));
        dir
    }

    pub fn queue_dir(&self, core_id: CoreId) -> PathBuf {
        let mut dir = self.output_dir(core_id);
        dir.push("queue");
        dir
    }

    pub fn work_dir(&self) -> PathBuf {
        let mut dir = PathBuf::from(&self.output);
        dir.push("workdir");
        dir
    }

    pub fn crashes_dir(&self, core_id: CoreId) -> PathBuf {
        let mut dir = self.output_dir(core_id);
        dir.push("crashes");
        dir
    }

    /// The weight for a mutator or generator, or 0.0 if it is disabled.
    pub fn mutator_weight<R: RngExt>(&self, name: &str, weight: f32, rng: &mut R) -> f32 {
        let base_weight = match &self.mutators {
            Some(list) if !list.iter().any(|m| m == name) => 0.0,
            _ => weight,
        };

        if self.swarm < 1.0 && base_weight > 0.0 && !rng.random_bool(self.swarm) {
            0.0
        } else {
            base_weight
        }
    }
}
