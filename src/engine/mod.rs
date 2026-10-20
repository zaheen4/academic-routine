//! Core scheduling engine, ingestion parsers, and analyzers.

pub mod parser;

pub use parser::{load_routine_file, parse_routine_json, IngestError};
