//! A failure saved with everything a replay needs.
//!
//! An artifact is a versioned envelope around a program: the scenario it ran against, the
//! revisions of what it was built against, how a campaign came to it, and what the oracle
//! said. The program's own context carries the deployment's seed, and that is the only place
//! the seed is written, so an artifact cannot disagree with itself about it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Program;

/// Marks a file as an artifact rather than a bare program, which is what the CLI writes and
/// what a scenario binary also accepts.
const MAGIC: [u8; 8] = *b"stratamo";

/// The envelope's format.
///
/// Bumped whenever the envelope or the program it carries changes shape, so that an older file
/// fails with its version rather than as garbage. The header that carries it, the marker then
/// the version, never changes shape itself.
pub const VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// What was run. Its context carries the deployment's seed.
    pub program: Program,
    /// The scenario it ran against, by name.
    pub scenario: String,
    /// What the harness was built against: the roles under test, their launchers, the protocol
    /// library both sides share, and the harness itself.
    pub revisions: Vec<Revision>,
    /// How the program was found, when a campaign found it rather than a hand.
    pub campaign: Option<Campaign>,
    /// What the oracle said.
    pub verdict: String,
    /// Whether the program drew the verdict again when run once more from a clean start.
    pub confirmed: bool,
    /// The record of the run that drew the verdict, as the harness that wrote the artifact
    /// encodes it: the postcard encoding of its execution type,
    /// which this crate does not know. The program is what replays; this is what was seen.
    pub trace: Option<Vec<u8>>,
}

/// A crate the harness was built against, as the lock file pins it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub name: String,
    pub version: String,
    /// Where it came from, with the exact commit for a git dependency.
    pub source: String,
}

/// The campaign that found a program.
///
/// Against a real target the program's seed reproduces nothing, since the target does not run
/// on the simulator; the campaign's seed and the target's revision are what reproduce the search
/// that found it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Campaign {
    /// The seed of the campaign's random source, which drives generation and mutation.
    pub seed: u64,
    /// The iteration the program was run at; zero for one found while seeding the corpus.
    pub iteration: u64,
    /// The corpus entry it was mutated from, copied in so that a replay does not depend on the
    /// corpus. None for a seed program, which was generated rather than mutated.
    pub parent: Option<Program>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    magic: [u8; 8],
    version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    /// The bytes do not start with the artifact marker.
    NotAnArtifact,
    /// The artifact was written by another version of the envelope.
    UnsupportedVersion { found: u32, supported: u32 },
    /// The marker and version are right and the rest is not.
    Corrupt(String),
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactError::NotAnArtifact => write!(f, "not a stratamoto artifact"),
            ArtifactError::UnsupportedVersion { found, supported } => write!(
                f,
                "artifact version {found} is not supported; this build reads version {supported}"
            ),
            ArtifactError::Corrupt(e) => write!(f, "corrupt artifact: {e}"),
        }
    }
}

impl std::error::Error for ArtifactError {}

impl Artifact {
    pub fn encode(&self) -> Result<Vec<u8>, ArtifactError> {
        let header = Header {
            magic: MAGIC,
            version: VERSION,
        };
        let mut bytes =
            postcard::to_allocvec(&header).map_err(|e| ArtifactError::Corrupt(e.to_string()))?;
        bytes.extend(
            postcard::to_allocvec(self).map_err(|e| ArtifactError::Corrupt(e.to_string()))?,
        );
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let (header, body): (Header, &[u8]) =
            postcard::take_from_bytes(bytes).map_err(|_| ArtifactError::NotAnArtifact)?;
        if header.magic != MAGIC {
            return Err(ArtifactError::NotAnArtifact);
        }
        if header.version != VERSION {
            return Err(ArtifactError::UnsupportedVersion {
                found: header.version,
                supported: VERSION,
            });
        }
        postcard::from_bytes(body).map_err(|e| ArtifactError::Corrupt(e.to_string()))
    }
}

/// A program from either form it is written in: an artifact, or a bare program as the CLI
/// writes it.
pub fn read_program(bytes: &[u8]) -> Result<Program, ArtifactError> {
    match Artifact::decode(bytes) {
        Ok(artifact) => Ok(artifact.program),
        Err(ArtifactError::NotAnArtifact) => {
            postcard::from_bytes(bytes).map_err(|e| ArtifactError::Corrupt(e.to_string()))
        }
        Err(e) => Err(e),
    }
}
