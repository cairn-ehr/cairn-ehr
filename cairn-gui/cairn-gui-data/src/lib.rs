pub mod mock;
pub mod port;
pub use mock::MockData;
pub use port::{ClinicalData, DataError, Demographics, NoteRef};
