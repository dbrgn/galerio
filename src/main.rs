use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{anyhow, Result};
use image::{self, imageops::FilterType, ImageFormat};
use lazy_static::lazy_static;
use serde::Serialize;
use structopt::StructOpt;
use tera::Tera;

const NAME: &str = "gallerist";
const VERSION: &str = env!("CARGO_PKG_VERSION");

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

    /// Gallery title
    title: String,

    /// Skip creating thumbnails
    #[structopt(long)]
    skip_thumbnails: bool,
}

#[derive(Serialize)]
struct Image {
    filename_full: String,
    filename_thumb: String,
}

#[derive(Serialize)]
struct Context {
    title: String,
    gallerist_version: &'static str,
    images: Vec<Image>,
}

/// Generate a thumbnail from the `image_path`, return the thumbnail bytes.
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
    let mut images = Vec::with_capacity(image_files.len());
    for f in image_files {
        // Determine filenames
        let filename_full = f.file_name().unwrap().to_str().unwrap().to_string();
        let filename_thumb = format!(
            "{}.thumb.jpg",
            f.file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| anyhow!("Could not determine file stem for file {:?}", f))?,
        );

        // Resize
        if !args.skip_thumbnails {
            log!("Processing {:?}", filename_full);
            let thumbnail_bytes = make_thumbnail(&f, 512)?;
            let thumbnail_path = args.output_dir.join(&filename_thumb);
            fs::write(thumbnail_path, thumbnail_bytes)?;
        }

        // Store
        images.push(Image {
            filename_full,
            filename_thumb,
        });
    }

    // Create template context
    let context = Context {
        title: args.title.clone(),
        gallerist_version: VERSION,
        images,
    };

    // Generate index.html
    let tera = Tera::new("templates/**/*.html")?;
    let rendered = tera.render("index.html", &tera::Context::from_serialize(&context)?)?;
    log!("Writing index.html");
    fs::write(args.output_dir.join("index.html"), rendered)?;

    Ok(())
}
