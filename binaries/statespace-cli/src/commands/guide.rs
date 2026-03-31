use crate::error::Result;
use statespace_templates::AGENTS_MD;

pub(crate) fn run_guide() -> Result<()> {
    print!("{AGENTS_MD}");
    Ok(())
}
