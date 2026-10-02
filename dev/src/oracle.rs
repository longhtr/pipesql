//! Expose independent readers and reference calculations for development checks.
//!
//! Catalog and snapshot modules decode observable bytes without linking the engine.
//! Catalog vectors construct controlled stored records, while rounding derives
//! numeric expectations with exact arithmetic. Campaigns use these results to judge
//! stock-library drivers and CLI output. Independence ends if an expected answer is
//! computed by the production codec or calculation it is supposed to check.

pub mod catalog;
pub mod catalog_vectors;
pub mod rounding;
pub mod snapshot;
