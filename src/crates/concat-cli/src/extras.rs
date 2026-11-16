// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! `concat-cli extras`: making extras and repositories, and looking inside
//! them. See docs/development/EXTRAS_PACKAGE_FORMAT.md.

use std::error::Error;
use std::path::{Path, PathBuf};

use clap::Subcommand;

/// The `extras` subcommands.
#[derive(Subcommand)]
pub enum ExtrasCommand {
    /// Make a signing key for a repository. The secret is written to
    /// `secret` (keep it private, out of every repository); the public key
    /// is printed, for `repositories.toml` or the build's official key.
    Keygen {
        /// Where to write the secret key. Refuses to overwrite one.
        secret: PathBuf,
    },
    /// Pack a folder - its `extra.toml` and the files beside it - into a
    /// `.concatpkg`, listing every file with its size and SHA-256.
    Pack {
        /// The folder with `extra.toml` in it.
        folder: PathBuf,
        /// The archive to write; `<id>-<version>.concatpkg` beside the
        /// folder when not given.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build a repository: pack every source folder, write `index.json`
    /// listing them, and sign it into `index.json.sig`.
    Repo {
        /// Where the repository is written.
        out: PathBuf,
        /// The secret key from `keygen`.
        #[arg(long)]
        key: PathBuf,
        /// The repository's name, as the Extras sheet shows it.
        #[arg(long, default_value = "My extras")]
        name: String,
        /// The index's revision; the date and time when not given.
        #[arg(long)]
        revision: Option<String>,
        /// How many days the signed index is believed for; Concat refuses
        /// it after that, so the repository must publish again before then.
        /// 0 for an index that never expires (not recommended).
        #[arg(long, default_value_t = 365)]
        valid_days: u64,
        /// The index's serial, which every publish must raise; readers
        /// refuse an index older than one they have seen. Now, in Unix
        /// seconds, when not given.
        #[arg(long)]
        serial: Option<u64>,
        /// The extras' source folders, each with an `extra.toml`.
        #[arg(required = true)]
        sources: Vec<PathBuf>,
    },
    /// Check a repository's index against its signature and a public key,
    /// and list what it offers.
    Verify {
        /// The `index.json`; its `.sig` is read from beside it.
        index: PathBuf,
        /// The public key, hex.
        #[arg(long)]
        public_key: String,
    },
    /// Check an archive exactly as an install would - every member, size
    /// and digest - without installing it, and print its manifest.
    Inspect {
        /// The `.concatpkg`.
        archive: PathBuf,
    },
}

/// Writes a secret key to a new file: never over one already there (the
/// check and the create are one step, so nothing can slip in between), and
/// on Unix readable by its owner alone. On Windows the file takes its
/// folder's permissions; keep it in a folder only you can read.
fn write_secret(path: &std::path::Path, secret: &str) -> Result<(), Box<dyn Error>> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            format!("{} is already there; it was left as it is", path.display())
        } else {
            format!("could not write {}: {error}", path.display())
        }
    })?;
    file.write_all(secret.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Runs one `extras` subcommand.
pub fn run(command: ExtrasCommand) -> Result<(), Box<dyn Error>> {
    match command {
        ExtrasCommand::Keygen { secret } => {
            let (private, public) = concat_extras::pack::keygen()?;
            write_secret(&secret, &private)?;
            println!("secret key written to {}", secret.display());
            println!("public key {public}");
            Ok(())
        }
        ExtrasCommand::Pack { folder, out } => {
            let out = match out {
                Some(out) => out,
                None => default_out(&folder)?,
            };
            let packed = concat_extras::pack::pack(&folder, &out)?;
            println!(
                "{} {} · {} files · {} → {}",
                packed.manifest.extra.id,
                packed.manifest.extra.version,
                packed.manifest.files.len(),
                concat_extras::human_size(packed.size),
                out.display()
            );
            println!("sha256 {}", packed.sha256);
            Ok(())
        }
        ExtrasCommand::Repo {
            out,
            key,
            name,
            revision,
            valid_days,
            serial,
            sources,
        } => {
            let secret = std::fs::read_to_string(&key)?;
            let now = concat_extras::index::unix_now();
            let serial = serial.unwrap_or(now);
            let expires = if valid_days == 0 {
                0
            } else {
                now + valid_days * 86_400
            };
            let revision = revision.unwrap_or_else(|| {
                let seconds = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                seconds.to_string()
            });
            let packed = concat_extras::pack::build_repository_with(
                &sources,
                &out,
                &name,
                &revision,
                secret.trim(),
                serial,
                expires,
            )?;
            for one in &packed {
                println!("{} {}", one.manifest.extra.id, one.manifest.extra.version);
            }
            println!(
                "{} extras in {} · serial {serial} · {} · public key {}",
                packed.len(),
                out.display(),
                if expires == 0 {
                    "never expires".to_owned()
                } else {
                    format!("expires in {valid_days} days")
                },
                concat_extras::pack::public_key(secret.trim())?
            );
            Ok(())
        }
        ExtrasCommand::Verify { index, public_key } => {
            let bytes = std::fs::read(&index)?;
            let sig = PathBuf::from(format!("{}.sig", index.display()));
            let signature = std::fs::read_to_string(&sig)?;
            let parsed =
                concat_extras::Index::verify_and_parse(&bytes, signature.trim(), &public_key)?;
            println!(
                "{} · revision {} · serial {} · {} · signature good",
                parsed.repository,
                parsed.revision,
                parsed.serial,
                if parsed.expires == 0 {
                    "never expires".to_owned()
                } else {
                    format!(
                        "expires in {} days",
                        parsed
                            .expires
                            .saturating_sub(concat_extras::index::unix_now())
                            / 86_400
                    )
                }
            );
            for entry in &parsed.extras {
                let fits = match entry.compatible(
                    concat_extras::APP_VERSION,
                    &concat_extras::current_platform(),
                ) {
                    Ok(()) => String::new(),
                    Err(why) => format!(" (not for this Concat: {why})"),
                };
                println!(
                    "  {} {} · {} · {}{fits}",
                    entry.id,
                    entry.version,
                    entry.category.key(),
                    concat_extras::human_size(entry.download.size)
                );
            }
            Ok(())
        }
        ExtrasCommand::Inspect { archive } => {
            let scratch =
                std::env::temp_dir().join(format!("concat-inspect-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&scratch);
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let result =
                concat_extras::archive::extract(&archive, &scratch, &cancel, &mut |_, _| {});
            let _ = std::fs::remove_dir_all(&scratch);
            let manifest = result?;
            let sha = concat_extras::archive::sha256_file(&archive, &cancel)?;
            println!(
                "{} {} by {} · every member checks out",
                manifest.extra.id, manifest.extra.version, manifest.extra.author
            );
            println!("sha256 {sha}");
            for provide in &manifest.provides {
                println!("  provides {} from {}/", provide.kind.key(), provide.root);
            }
            for requirement in &manifest.requires {
                println!("  requires {} {}", requirement.id, requirement.version);
            }
            println!(
                "  {} files, {}",
                manifest.files.len(),
                concat_extras::human_size(manifest.total_size())
            );
            Ok(())
        }
    }
}

fn default_out(folder: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let text = std::fs::read_to_string(folder.join(concat_extras::manifest::MANIFEST_FILE))?;
    let manifest: concat_extras::Manifest = toml::from_str(&text)?;
    let name = format!(
        "{}-{}.{}",
        manifest.extra.id,
        manifest.extra.version,
        concat_extras::archive::EXTENSION
    );
    let parent = folder
        .canonicalize()
        .ok()
        .and_then(|folder| folder.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::write_secret;

    /// A secret key is written once, to a new file, and never over another.
    #[test]
    fn a_secret_key_is_never_written_over_one_already_there() {
        let folder = std::env::temp_dir().join(format!("concat-cli-secret-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("repo.key");
        let _ = std::fs::remove_file(&path);
        write_secret(&path, "first").unwrap();
        let error = write_secret(&path, "second").unwrap_err();
        assert!(error.to_string().contains("already there"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "readable by its owner alone");
        }
        let _ = std::fs::remove_dir_all(&folder);
    }
}
