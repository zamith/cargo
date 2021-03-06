//!
//! Cargo compile currently does the following steps:
//!
//! All configurations are already injected as environment variables via the
//! main cargo command
//!
//! 1. Read the manifest
//! 2. Shell out to `cargo-resolve` with a list of dependencies and sources as
//!    stdin
//!
//!    a. Shell out to `--do update` and `--do list` for each source
//!    b. Resolve dependencies and return a list of name/version/source
//!
//! 3. Shell out to `--do download` for each source
//! 4. Shell out to `--do get` for each source, and build up the list of paths
//!    to pass to rustc -L
//! 5. Call `cargo-rustc` with the results of the resolver zipped together with
//!    the results of the `get`
//!
//!    a. Topologically sort the dependencies
//!    b. Compile each dependency in order, passing in the -L's pointing at each
//!       previously compiled dependency
//!

use std::os;
use util::config::{Config, ConfigValue};
use core::{MultiShell, Source, SourceId, PackageSet, Target, PackageId, resolver};
use core::registry::PackageRegistry;
use ops;
use sources::{PathSource};
use util::{CargoResult, Wrap, config, internal, human};

pub struct CompileOptions<'a> {
    pub update: bool,
    pub env: &'a str,
    pub shell: &'a mut MultiShell,
    pub jobs: Option<uint>,
    pub target: Option<&'a str>,
}

pub fn compile(manifest_path: &Path, options: CompileOptions) -> CargoResult<Vec<String>> {
    let CompileOptions { update, env, shell, jobs, target } = options;
    let target = target.map(|s| s.to_string());

    log!(4, "compile; manifest-path={}", manifest_path.display());

    let mut source = PathSource::for_path(&manifest_path.dir_path());

    try!(source.update());

    // TODO: Move this into PathSource
    let package = try!(source.get_root_package());
    debug!("loaded package; package={}", package);

    for key in package.get_manifest().get_unused_keys().iter() {
        try!(shell.warn(format!("unused manifest key: {}", key)));
    }

    let override_ids = try!(source_ids_from_config());
    let source_ids = package.get_source_ids();

    let (packages, resolve) = {
        let mut config = try!(Config::new(shell, update, jobs, target.clone()));

        let mut registry =
            try!(PackageRegistry::new(source_ids, override_ids, &mut config));

        let resolved = try!(resolver::resolve(package.get_package_id(),
                                              package.get_dependencies(),
                                              &mut registry));

        let req: Vec<PackageId> = resolved.iter().map(|r| r.clone()).collect();
        let packages = try!(registry.get(req.as_slice()).wrap({
            human("Unable to get packages from source")
        }));

        (packages, resolved)
    };

    debug!("packages={}", packages);

    let targets = package.get_targets().iter().filter(|target| {
        target.get_profile().get_env() == env
    }).collect::<Vec<&Target>>();

    let mut config = try!(Config::new(shell, update, jobs, target));

    try!(ops::compile_targets(env.as_slice(), targets.as_slice(), &package,
         &PackageSet::new(packages.as_slice()), &resolve, &mut config));

    let test_executables: Vec<String> = targets.iter()
        .filter_map(|target| {
            if target.get_profile().is_test() {
                debug!("Run  Target: {}", target.get_name());
                Some(target.get_name().to_string())
            } else {
                debug!("Skip Target: {}", target.get_name());
                None
            }
    }).collect();

    Ok(test_executables)
}

fn source_ids_from_config() -> CargoResult<Vec<SourceId>> {
    let configs = try!(config::all_configs(os::getcwd()));

    debug!("loaded config; configs={}", configs);

    let config_paths = configs.find_equiv(&"paths").map(|v| v.clone());
    let config_paths = config_paths.unwrap_or_else(|| ConfigValue::new());

    let paths: Vec<Path> = match *config_paths.get_value() {
        config::String(_) => return Err(internal("The path was configured as \
                                                   a String instead of a List")),
        config::List(ref list) => {
            list.iter().map(|path| Path::new(path.as_slice())).collect()
        }
    };

    Ok(paths.iter().map(|p| SourceId::for_path(p)).collect())
}
