use stratamoto_ir::{
    Program, ProgramBuilder, ProgramContext,
    artifact::{Artifact, ArtifactError, Campaign, Revision, VERSION, read_program},
};

fn program(seed: u64) -> Program {
    ProgramBuilder::new(ProgramContext {
        num_roles: 3,
        num_connections: 0,
        seed,
    })
    .finalize()
    .unwrap()
}

fn artifact() -> Artifact {
    Artifact {
        program: program(7),
        scenario: "setup_connection".to_string(),
        revisions: vec![Revision {
            name: "stratum-core".to_string(),
            version: "0.6.0".to_string(),
            source: "git+https://example.invalid/stratum#abc".to_string(),
        }],
        campaign: Some(Campaign {
            seed: 1,
            iteration: 42,
            parent: Some(program(7)),
        }),
        verdict: "setup_connection: no answer".to_string(),
        confirmed: true,
        trace: Some(vec![1, 2, 3]),
    }
}

#[test]
fn an_artifact_survives_a_round_trip() {
    let artifact = artifact();
    let bytes = artifact.encode().unwrap();
    assert_eq!(Artifact::decode(&bytes).unwrap(), artifact);
}

/// The seed lives in the program and nowhere else in the envelope, so replaying reads it from
/// there.
#[test]
fn the_seed_is_read_from_the_program() {
    let bytes = artifact().encode().unwrap();
    assert_eq!(read_program(&bytes).unwrap().context.seed, 7);
}

#[test]
fn a_bare_program_is_still_readable() {
    let bytes = postcard::to_allocvec(&program(3)).unwrap();
    assert_eq!(Artifact::decode(&bytes), Err(ArtifactError::NotAnArtifact));
    assert_eq!(read_program(&bytes).unwrap(), program(3));
}

#[test]
fn another_version_fails_by_name() {
    let mut bytes = b"stratamo".to_vec();
    bytes.extend(postcard::to_allocvec(&(VERSION + 1)).unwrap());
    let expected = ArtifactError::UnsupportedVersion {
        found: VERSION + 1,
        supported: VERSION,
    };
    assert_eq!(Artifact::decode(&bytes), Err(expected.clone()));
    assert_eq!(read_program(&bytes), Err(expected));
}

#[test]
fn garbage_is_neither() {
    assert_eq!(Artifact::decode(b"nope"), Err(ArtifactError::NotAnArtifact));
    assert!(matches!(
        read_program(b"nope"),
        Err(ArtifactError::Corrupt(_))
    ));

    let mut truncated = artifact().encode().unwrap();
    truncated.truncate(12);
    assert!(matches!(
        Artifact::decode(&truncated),
        Err(ArtifactError::Corrupt(_))
    ));
}
