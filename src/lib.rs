use std::{env, fs};

use zed_extension_api::{self as zed, Result};

const SERVER_ID: &str = "ejs-lsp";
const SERVER_DIR: &str = "ejs-lsp";
const GITHUB_REPO: &str = "binhphan7588/zed-ejs";
const PLACEHOLDER_GITHUB_REPO: &str = "";
const LOCAL_DEV_SERVER_PATH: &str = "C:/Users/Binh/Downloads/zed-ejs/ejs-lsp/ejs-lsp.exe";
const EMMET_SERVER_ID: &str = "emmet-language-server";
const EMMET_SERVER_PATH: &str = "node_modules/@olrtg/emmet-language-server/dist/index.js";
const EMMET_PACKAGE_NAME: &str = "@olrtg/emmet-language-server";

struct EjsExtension {
    did_find_server: bool,
    did_find_emmet_server: bool,
}

impl EjsExtension {
    fn server_path(&mut self, language_server_id: &zed::LanguageServerId) -> Result<String> {
        if language_server_id.as_ref() != SERVER_ID {
            return Err(format!(
                "unknown language server: {}",
                language_server_id.as_ref()
            ));
        }

        let extension_dir = env::current_dir()
            .map_err(|err| format!("failed to read extension working directory: {err}"))?;
        let bundled = extension_dir.join(SERVER_DIR).join(executable_name());

        if bundled.is_file() {
            self.did_find_server = true;
            return Ok(bundled.to_string_lossy().to_string());
        }

        let dev = extension_dir
            .join("crates")
            .join("ejs-lsp")
            .join("target")
            .join("release")
            .join(executable_name());

        if dev.is_file() {
            self.did_find_server = true;
            return Ok(dev.to_string_lossy().to_string());
        }

        if zed::current_platform().0 == zed::Os::Windows {
            let local_dev = std::path::Path::new(LOCAL_DEV_SERVER_PATH);
            if local_dev.is_file() {
                self.did_find_server = true;
                return Ok(local_dev.to_string_lossy().to_string());
            }
        }

        if self.did_find_server {
            return Ok(bundled.to_string_lossy().to_string());
        }

        if GITHUB_REPO == PLACEHOLDER_GITHUB_REPO {
            return Err(format!(
                "EJS language server binary was not found at {} or {}. Build it with `cargo build --release --manifest-path crates/ejs-lsp/Cargo.toml`, then copy it to {SERVER_DIR}/{} for release packaging, or keep it at the local dev path.",
                bundled.display(),
                LOCAL_DEV_SERVER_PATH,
                executable_name()
            ));
        }

        self.download_server(language_server_id)?;
        if bundled.is_file() {
            self.did_find_server = true;
            Ok(bundled.to_string_lossy().to_string())
        } else {
            Err(format!(
                "downloaded EJS language server did not contain {}",
                bundled.display()
            ))
        }
    }

    fn download_server(&self, language_server_id: &zed::LanguageServerId) -> Result<()> {
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            GITHUB_REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;
        let asset_name = asset_name();
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "no asset named '{asset_name}' found in latest release {} of {GITHUB_REPO}",
                    release.version
                )
            })?;

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );
        zed::download_file(
            &asset.download_url,
            SERVER_DIR,
            zed::DownloadedFileType::Zip,
        )?;

        let (os, _) = zed::current_platform();
        let path = format!("{SERVER_DIR}/{}", executable_name());
        if os != zed::Os::Windows {
            zed::make_file_executable(&path)?;
        }
        Ok(())
    }

    fn emmet_server_exists(&self) -> bool {
        fs::metadata(EMMET_SERVER_PATH).is_ok_and(|stat| stat.is_file())
    }

    fn emmet_server_script_path(&mut self, language_server_id: &zed::LanguageServerId) -> Result<String> {
        let server_exists = self.emmet_server_exists();
        if self.did_find_emmet_server && server_exists {
            return Ok(EMMET_SERVER_PATH.to_string());
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let version = zed::npm_package_latest_version(EMMET_PACKAGE_NAME)?;

        if !server_exists
            || zed::npm_package_installed_version(EMMET_PACKAGE_NAME)?.as_ref() != Some(&version)
        {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            let result = zed::npm_install_package(EMMET_PACKAGE_NAME, &version);
            match result {
                Ok(()) => {
                    if !self.emmet_server_exists() {
                        return Err(format!(
                            "installed package '{EMMET_PACKAGE_NAME}' did not contain expected path '{EMMET_SERVER_PATH}'"
                        ));
                    }
                }
                Err(error) => {
                    if !self.emmet_server_exists() {
                        return Err(error);
                    }
                }
            }
        }

        self.did_find_emmet_server = true;
        Ok(EMMET_SERVER_PATH.to_string())
    }
}

impl zed::Extension for EjsExtension {
    fn new() -> Self {
        Self {
            did_find_server: false,
            did_find_emmet_server: false,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        _worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        if language_server_id.as_ref() == EMMET_SERVER_ID {
            let server_path = self.emmet_server_script_path(language_server_id)?;
            return Ok(zed::Command {
                command: zed::node_binary_path()?,
                args: vec![
                    env::current_dir()
                        .map_err(|err| format!("failed to read extension working directory: {err}"))?
                        .join(server_path)
                        .to_string_lossy()
                        .to_string(),
                    "--stdio".to_string(),
                ],
                env: Vec::new(),
            });
        }

        Ok(zed::Command {
            command: self.server_path(language_server_id)?,
            args: Vec::new(),
            env: Vec::new(),
        })
    }
}

fn executable_name() -> &'static str {
    match zed::current_platform().0 {
        zed::Os::Windows => "ejs-lsp.exe",
        zed::Os::Mac | zed::Os::Linux => "ejs-lsp",
    }
}

fn asset_name() -> String {
    let (os, arch) = zed::current_platform();
    let os = match os {
        zed::Os::Mac => "darwin",
        zed::Os::Linux => "linux",
        zed::Os::Windows => "windows",
    };
    let arch = match arch {
        zed::Architecture::Aarch64 => "aarch64",
        zed::Architecture::X86 => "x86",
        zed::Architecture::X8664 => "x86_64",
    };
    format!("ejs-lsp-{os}-{arch}.zip")
}

zed::register_extension!(EjsExtension);
