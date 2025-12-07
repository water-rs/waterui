use std::path::Path;

use color_eyre::eyre;

use crate::project::Project;

pub struct BuildOptions {}

pub struct PackageOptions {}

pub trait Platform {
    // build rust code for this platform
    fn build(&self, project: &Project, options: &BuildOptions) -> eyre::Result<Path>;

    fn package(&self, project: &Project, options: &PackageOptions) -> eyre::Result<Path>;

    fn description(&self) -> String;
}
