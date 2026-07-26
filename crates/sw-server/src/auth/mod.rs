//! Auth boundaries. The shell defines extractors and JWT helpers; it does not
//! implement wallet login or session issuance.

mod claims;
mod extractor;

pub use claims::*;
pub use extractor::*;
