//! CPF machine management. Console execution lives behind runtime adapters.
mod machine;
mod cartridge;
mod preservation;
mod runtime;
mod storage;
mod verify;

pub use machine::{
    hash_file, read_json, MachineDetail, MachineSettings, MachineSummary, Readiness, Workspace,
};
pub use cartridge::{inspect_nds, CartridgeInfo};
pub use preservation::{CleanNandResult, CloneResult, RestoredFile, SaveImportResult};
pub use runtime::{LaunchResult, RuntimeAdapter, RuntimeRegistry};
pub use storage::{SdImportResult, SdRestoreResult, SdSnapshot, SdSnapshotSummary, StorageComparison};
pub use verify::{VerificationCheck, VerificationReport};
