use crate::coverage;
use crate::devcontainer::run_in_devcontainer;
use crate::{CoverArgs, DocsArgs};
use anyhow::Result;

/// Reproduce the full proof chain inside the canonical devcontainer.
///
/// The host enters the container once, then the container runs CI, coverage,
/// and docs in-process so we do not pay repeated container-entry ceremony for
/// one logical proof.
pub(crate) fn preflight() -> Result<()> {
    if std::env::var_os("DEVCONTAINER").is_some() {
        return preflight_inner();
    }

    run_in_devcontainer(&["cargo", "xtask", "preflight"])
}

fn preflight_inner() -> Result<()> {
    crate::commands::ci()?;
    coverage::cover(CoverArgs {
        ci: true,
        json: false,
        threshold: Some(80),
    })?;
    crate::docs::docs(DocsArgs { open: false })
}
