use crate::error::{Error, Result};

const DOCS_URL: &str = "https://docs.statespace.com";

pub(crate) fn run_docs() -> Result<()> {
    eprintln!("Opening {DOCS_URL}...");
    open::that(DOCS_URL).map_err(|e| Error::cli(format!("failed to open browser: {e}")))
}
