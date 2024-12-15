use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use std::{io::Write, path::PathBuf, process::Command};
use semver::Version;

#[derive(Parser, Debug)]
#[command(about = "Downloads the latest PowerShell and UV for Windows x64")]
struct Args {
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

struct Tool {
    name: String,
    repo: String,
    exe: String,
    version_pattern: &'static str,
}

impl Tool {
    fn powershell() -> Self {
        Self {
            name: "PowerShell".to_string(),
            repo: "PowerShell/PowerShell".to_string(),
            exe: "pwsh.exe".to_string(),
            version_pattern: r"PowerShell ([\d\.]+)",
        }
    }

    fn uv() -> Self {
        Self {
            name: "UV".to_string(),
            repo: "astral-sh/uv".to_string(),
            exe: "uv.exe".to_string(),
            version_pattern: r"uv ([\d\.]+)",
        }
    }

    fn matches_asset(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        match self.name.as_str() {
            "PowerShell" => name.contains("win") && name.contains("x64") && 
                           name.ends_with(".zip") && !name.contains("symbols") && 
                           !name.contains("arm"),
            "UV" => name.contains("windows") && name.contains("x86_64") && 
                    name.ends_with(".zip"),
            _ => false,
        }
    }
}

fn create_progress_bar(len: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) • {msg}")
        .unwrap()
        .progress_chars("#>-"));
    pb.set_message(message.to_string());
    pb
}

fn process_tool(client: &Client, tool: &Tool, dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!("\nChecking {} installation...", tool.name);
    
    // Check current version
    let exe_path = dir.join(&tool.exe);
    let current_version = if exe_path.exists() {
        Command::new(&exe_path)
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                let version_str = String::from_utf8_lossy(&output.stdout);
                regex::Regex::new(tool.version_pattern)
                    .ok()?
                    .captures(&version_str)?
                    .get(1)?
                    .as_str()
                    .parse::<Version>()
                    .ok()
            })
    } else {
        None
    };

    // Get latest version
    let release: Release = client
        .get(&format!("https://api.github.com/repos/{}/releases/latest", tool.repo))
        .send()?
        .json()?;
    
    let latest_version = Version::parse(release.tag_name.trim_start_matches('v'))?;
    
    // Check if update needed
    if let Some(ver) = current_version {
        println!("Installed version: {}", ver);
        println!("Latest version: {}", latest_version);
        if ver >= latest_version {
            println!("{} is up to date!", tool.name);
            return Ok(());
        }
        println!("Update available!");
    } else {
        println!("Not installed or version check failed.");
    }

    // Find and download asset
    let asset = release.assets.iter()
        .find(|a| tool.matches_asset(&a.name))
        .ok_or("Compatible release not found")?;

    let zip_path = dir.join(&asset.name);

    // Download with progress
    let response = client.get(&asset.browser_download_url).send()?;
    let pb = create_progress_bar(
        response.content_length().unwrap_or(0),
        &format!("Downloading {}", tool.name)
    );

    let bytes = response.bytes()?;
    std::fs::File::create(&zip_path)?.write_all(&bytes)?;
    pb.inc(bytes.len() as u64);
    pb.finish();

    // Extract with progress
    let pb = create_progress_bar(0, &format!("Extracting {}", tool.name));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path)?)?;
    pb.set_length(archive.len() as u64);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dir.join(path),
            None => continue,
        };

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(p)?;
                }
            }
            std::io::copy(&mut file, &mut std::fs::File::create(&outpath)?)?;
        }
        pb.inc(1);
    }
    pb.finish();

    std::fs::remove_file(zip_path)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let base_dir = match args.output {
        Some(dir) => dir,
        None => std::env::current_dir()?
    };
    println!("Output directory: {}", base_dir.display());

    let pwsh_dir = base_dir.join("pwsh");
    let uv_dir = base_dir.join("uv");
    std::fs::create_dir_all(&pwsh_dir)?;
    std::fs::create_dir_all(&uv_dir)?;

    let client = Client::builder()
        .default_headers({
            let mut headers = HeaderMap::new();
            headers.insert(USER_AGENT, HeaderValue::from_static("GitHub-Tool-Downloader-Rust"));
            headers
        })
        .build()?;

    process_tool(&client, &Tool::powershell(), &pwsh_dir)?;
    process_tool(&client, &Tool::uv(), &uv_dir)?;

    println!("\nAll tools are up to date!");
    println!("PowerShell: {}", pwsh_dir.join("pwsh.exe").display());
    println!("UV: {}", uv_dir.join("uv.exe").display());

    Ok(())
}