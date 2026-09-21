use std::io::{Read, Write};

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
";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("generate") => generate(&args[1..]),
        Some("print") => print(),
        Some("compile") => compile(),
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

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).map_err(err)?;
    Ok(bytes)
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
