use std::io::{Read, Write};

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    Program, ProgramBuilder, ProgramContext,
    compiler::Compiler,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
};

const USAGE: &str = "\
usage: stratamoto <command>

  generate [seed] [roles]   write a program for the given seed to stdout
  print                     pretty print a program read from stdin
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

    let generator = SetupConnectionGenerator {
        role: (seed as usize) % num_roles,
        protocol: None,
    };
    generator.generate(&mut builder, &mut rng).map_err(err)?;

    let program = builder.finalize().map_err(err)?;
    let bytes = postcard::to_allocvec(&program).map_err(err)?;
    std::io::stdout().write_all(&bytes).map_err(err)
}

fn print() -> Result<(), String> {
    print!("{}", read_program()?);
    Ok(())
}

fn compile() -> Result<(), String> {
    let program = read_program()?;
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

fn read_program() -> Result<Program, String> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).map_err(err)?;
    postcard::from_bytes(&bytes).map_err(err)
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
