mod init;

use std::{
    io::{Read, Write},
    path::PathBuf,
};

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    ProgramBuilder, ProgramContext,
    artifact::{Artifact, ArtifactError, read_program},
    compiler::Compiler,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
};

const USAGE: &str = "\
usage: stratamoto <command>

  generate [seed] [roles]   write a program for the given seed to stdout
  print                     pretty print a program, or an artifact, read from stdin
  compile                   print the actions a program from stdin compiles to
  init <options>            create a Nyx share directory for a scenario:
    --sharedir <dir>            where to create it (must not exist)
    --scenario <bin>            the scenario binary, built with --features nyx
    --pool <bin>                the pool binary
    --template-provider <dir>   where Bitcoin Core and sv2-tp live
    --nyx-dir <dir>             AFL++'s nyx_mode, or a target dir libafl_nyx built into
    --crash-handler <so>        the handler to preload into the pool (default: the built one)
    --memory <MB>               the VM's memory (default: 4096)
";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("generate") => generate(&args[1..]),
        Some("print") => print(),
        Some("compile") => compile(),
        Some("init") => init_args(&args[1..]).and_then(|args| init::execute(&args)),
        _ => {
            eprint!("{USAGE}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn generate(args: &[String]) -> Result<(), String> {
    let seed: u64 = args.first().map_or(Ok(0), |s| s.parse().map_err(err))?;
    let num_roles: usize = args.get(1).map_or(Ok(3), |s| s.parse().map_err(err))?;

    let mut rng = SmallRng::seed_from_u64(seed);
    let mut builder = ProgramBuilder::new(ProgramContext {
        num_roles,
        num_connections: 0,
        seed,
    });

    let generator = SetupConnectionGenerator::new((seed as usize) % num_roles);
    generator.generate(&mut builder, &mut rng).map_err(err)?;

    let program = builder.finalize().map_err(err)?;
    let bytes = postcard::to_allocvec(&program).map_err(err)?;
    std::io::stdout().write_all(&bytes).map_err(err)
}

fn print() -> Result<(), String> {
    let bytes = read_stdin()?;
    match Artifact::decode(&bytes) {
        Ok(artifact) => print_artifact(&artifact),
        Err(ArtifactError::NotAnArtifact) => print!("{}", read_program(&bytes).map_err(err)?),
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// The envelope as comment lines the program's own header already uses, then the program,
/// then what it was mutated from.
fn print_artifact(artifact: &Artifact) {
    println!("// scenario={}", artifact.scenario);
    println!("// verdict={}", artifact.verdict);
    println!(
        "// confirmed={} trace={}",
        artifact.confirmed,
        artifact
            .trace
            .as_ref()
            .map_or("none".to_string(), |t| format!("{} bytes", t.len()))
    );
    if let Some(campaign) = &artifact.campaign {
        println!(
            "// campaign seed={} iteration={}",
            campaign.seed, campaign.iteration
        );
    }
    for revision in &artifact.revisions {
        println!(
            "// built against {} {} {}",
            revision.name, revision.version, revision.source
        );
    }
    print!("{}", artifact.program);
    if let Some(parent) = artifact.campaign.as_ref().and_then(|c| c.parent.as_ref()) {
        println!("// mutated from:");
        print!("{parent}");
    }
}

fn compile() -> Result<(), String> {
    let program = read_program(&read_stdin()?).map_err(err)?;
    let compiled = Compiler::new().compile(&program).map_err(err)?;
    for (action, instruction) in compiled
        .actions
        .iter()
        .zip(compiled.metadata.action_instructions.iter())
    {
        println!("[{instruction}] {action:?}");
    }
    Ok(())
}

/// `--key value` pairs into the init command's arguments.
fn init_args(args: &[String]) -> Result<init::InitArgs, String> {
    let mut sharedir = None;
    let mut scenario = None;
    let mut pool = None;
    let mut template_provider = None;
    let mut nyx_dir = None;
    let mut crash_handler = PathBuf::from(stratamoto_nyx_sys::CRASH_HANDLER);
    let mut memory = 4096;

    let mut pairs = args.chunks(2);
    for pair in &mut pairs {
        let [key, value] = pair else {
            return Err(format!("{} takes a value", pair[0]));
        };
        match key.as_str() {
            "--sharedir" => sharedir = Some(PathBuf::from(value)),
            "--scenario" => scenario = Some(PathBuf::from(value)),
            "--pool" => pool = Some(PathBuf::from(value)),
            "--template-provider" => template_provider = Some(PathBuf::from(value)),
            "--nyx-dir" => nyx_dir = Some(PathBuf::from(value)),
            "--crash-handler" => crash_handler = PathBuf::from(value),
            "--memory" => memory = value.parse().map_err(err)?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let required = |name: &str, value: Option<PathBuf>| {
        value.ok_or_else(|| format!("init needs {name}; see `stratamoto` for the options"))
    };
    Ok(init::InitArgs {
        sharedir: required("--sharedir", sharedir)?,
        scenario: required("--scenario", scenario)?,
        pool: required("--pool", pool)?,
        template_provider: required("--template-provider", template_provider)?,
        nyx_dir: required("--nyx-dir", nyx_dir)?,
        crash_handler,
        memory,
    })
}

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).map_err(err)?;
    Ok(bytes)
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
