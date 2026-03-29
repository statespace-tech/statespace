use crate::error::Result;

const AGENTS_MD: &str = include_str!("../../../../AGENTS.md");

pub(crate) fn run_guide() -> Result<()> {
    print!("{AGENTS_MD}");
    Ok(())
}
