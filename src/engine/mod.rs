//! Core scheduling engine, ingestion parsers, and analyzers.

pub mod parser;
pub mod scraper;

pub use parser::{load_routine_file, parse_routine_json, IngestError};
pub use scraper::{run_ucam_scraper, run_ucam_scraper_opts, ScraperError};
