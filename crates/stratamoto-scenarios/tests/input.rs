use stratamoto::scenario::ScenarioInput;
use stratamoto_ir::{
    ProgramBuilder, ProgramContext,
    artifact::{Artifact, ArtifactError},
};
use stratamoto_scenarios::setup_connection::TestCase;

fn artifact(seed: u64) -> Artifact {
    let program = ProgramBuilder::new(ProgramContext {
        num_roles: 3,
        num_connections: 0,
        seed,
    })
    .finalize()
    .unwrap();
    Artifact {
        program,
        scenario: "setup_connection".to_string(),
        revisions: vec![],
        campaign: None,
        verdict: String::new(),
        confirmed: false,
        trace: None,
    }
}

/// A scenario binary replays a saved artifact as it is, and the seed it runs on is the one the
/// program carries.
#[test]
fn a_scenario_reads_an_artifact_and_its_seed() {
    let bytes = artifact(11).encode().unwrap();
    let testcase = TestCase::decode(&bytes).expect("an artifact is a scenario input");
    assert_eq!(testcase.program.context.seed, 11);
}

#[test]
fn a_scenario_still_reads_a_bare_program() {
    let bytes = postcard::to_allocvec(&artifact(5).program).unwrap();
    let testcase = TestCase::decode(&bytes).expect("a bare program is a scenario input");
    assert_eq!(testcase.program.context.seed, 5);
}

#[test]
fn a_scenario_names_an_unsupported_version() {
    let mut bytes = b"stratamo".to_vec();
    bytes.extend(postcard::to_allocvec(&u32::MAX).unwrap());
    let error = match TestCase::decode(&bytes) {
        Ok(_) => panic!("an unsupported version was accepted"),
        Err(e) => e.to_string(),
    };
    let expected = ArtifactError::UnsupportedVersion {
        found: u32::MAX,
        supported: stratamoto_ir::artifact::VERSION,
    };
    assert!(error.contains(&expected.to_string()), "{error}");
}
