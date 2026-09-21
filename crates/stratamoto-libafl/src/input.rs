use std::{hash::Hash, path::Path};

use libafl::inputs::{HasTargetBytes, Input};
use libafl_bolts::{HasLen, ownedref::OwnedSlice};
use stratamoto_ir::Program;

/// The largest input handed to the VM. Programs are small; one this size is a runaway.
const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// An IR program as the fuzzer's input.
///
/// Serialized, it is the program's own postcard encoding: a corpus entry is a file a scenario
/// binary reads and `stratamoto print` shows.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Hash)]
pub struct IrInput {
    ir: Program,
}

impl Input for IrInput {}

impl IrInput {
    pub fn new(ir: Program) -> Self {
        Self { ir }
    }

    pub fn ir(&self) -> &Program {
        &self.ir
    }

    pub fn ir_mut(&mut self) -> &mut Program {
        &mut self.ir
    }

    /// Read a program from `path`.
    pub fn unparse(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let ir = postcard::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { ir })
    }
}

impl HasLen for IrInput {
    fn len(&self) -> usize {
        self.ir().instructions.len()
    }
}

impl HasTargetBytes for IrInput {
    /// The program itself: the scenario compiles it inside the VM, which keeps the input
    /// buffer small and the corpus readable.
    fn target_bytes(&self) -> OwnedSlice<'_, u8> {
        let mut bytes = postcard::to_allocvec(self.ir()).expect("serialization never fails");
        tracing::trace!("input size: {}", bytes.len());
        if bytes.len() > MAX_INPUT_BYTES {
            bytes = Vec::new();
        }
        OwnedSlice::from(bytes)
    }
}
