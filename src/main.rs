use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Result};
use image::{self, imageops::FilterType, ImageFormat};
use lazy_static::lazy_static;
use structopt::StructOpt;

const NAME: &str = "gallerist";

lazy_static! {
    static ref START_TIME: Instant = Instant::now();
}

fn log(msg: &str) {
    let start_time = *START_TIME;
    let elapsed = Instant::now().duration_since(start_time).as_millis();
    println!("[+{:>4}ms] {}", elapsed, msg);
}

macro_rules! log {
    ($($arg:tt)*) => {
        log(&format!($($arg)*));
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = NAME)]
struct Args {
    /// Input directory
    #[structopt(parse(from_os_str))]
    input_dir: PathBuf,

    /// Output directory
    #[structopt(parse(from_os_str))]
    output_dir: PathBuf,
}

fn make_thumbnail(image_path: impl AsRef<Path>, thumbnail_size: u32) -> Result<Vec<u8>> {
    let img = image::open(image_path)?;
    let mut buf = Vec::new();
    let thumb = img.resize(thumbnail_size, thumbnail_size, FilterType::CatmullRom);
    thumb.write_to(&mut buf, ImageFormat::Jpeg)?;
    Ok(buf)
}

fn main() -> Result<()> {
    let args = Args::from_args();

    log!("Starting...");

    // Validate input directory path
    if !args.input_dir.exists() {
        return Err(anyhow!("Input directory does not exist"));
    }
    if !args.input_dir.is_dir() {
        return Err(anyhow!("Input directory path is not a directory"));
    }

    // Create output directory
    if !args.output_dir.exists() {
        log!("Creating output directory {:?}", args.output_dir);
        fs::create_dir_all(&args.output_dir)?;
    }

    log!("Input dir: {:?}", args.input_dir);
    log!("Output dir: {:?}", args.output_dir);

    // Get list of input images
    let image_files = fs::read_dir(args.input_dir)?
        .filter_map(|res| res.ok())
        .filter(|dir_entry| {
            dir_entry
                .file_type()
                .map(|ft| ft.is_file())
                .unwrap_or(false)
        })
        .filter(|dir_entry| {
            dir_entry
                .file_name()
                .to_str()
                .map(|s| s.ends_with(".jpg"))
                .unwrap_or(false)
        })
        .map(|dir_entry| dir_entry.path())
        .collect::<Vec<_>>();

    // Process images
    for f in image_files {
        log!("Processing {:?}", f.file_name().unwrap());
        let thumbnail_bytes = make_thumbnail(&f, 512)?;
        let thumbnail_path = args.output_dir.join(&format!(
            "{}.thumb.jpg",
            f.file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| anyhow!("Could not determine file stem for file {:?}", f))?,
        ));
        fs::write(thumbnail_path, thumbnail_bytes)?;
    }

    Ok(())
}
