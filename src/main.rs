use std::{
    fs,
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use exif::{In as IdfNum, Reader as ExifReader, Tag as ExifTag, Value as ExifValue};
use image::{self, imageops::FilterType, GenericImageView, ImageFormat};
use lazy_static::lazy_static;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use rust_embed::RustEmbed;
use serde::Serialize;
use structopt::StructOpt;
use tera::Tera;

const NAME: &str = "galerio";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(RustEmbed)]
#[folder = "templates/"]
struct Templates;

#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticFiles;

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

    /// Max thumbnail height in pixels
    #[structopt(short = "h", long = "height", default_value = "300")]
    thumbnail_height: u32,

    /// Max large image size in pixels
    #[structopt(short = "l", long = "max-large-size")]
    max_large_size: Option<u32>,

    /// Resize panoramas as well
    #[structopt(short = "p", long = "resize-include-panorama")]
    resize_include_panorama: bool,

    /// Disallow full gallery download as ZIP
    #[structopt(long = "no-download")]
    no_download: bool,

    /// Skip processing image files
    #[structopt(long)]
    skip_processing: bool,
}

#[derive(Serialize)]
struct Image {
    filename_full: String,
    filename_thumb: String,
}

#[derive(Serialize)]
struct TemplateContext {
    title: String,
    galerio_version: &'static str,
    isodate: String,
    download_filename: Option<String>,
    download_filesize_mib: Option<u64>,
    images: Vec<Image>,
}

/// Get the width and height of the image (whichever
fn get_dimensions(image_path: impl AsRef<Path>) -> Result<(u32, u32)> {
    let img = image::open(image_path)?;
    Ok(img.dimensions())
}

/// Generate a resized image from the `image_path`, return the resized bytes.
fn resize_image(
    image_path: impl AsRef<Path>,
    max_width: u32,
    max_height: u32,
    orientation: &Orientation,
    panorama_detection: bool,
) -> Result<Vec<u8>> {
    // Open original image
    let mut img = image::open(image_path)?;

    // Panorama detection: Aspect ratio more than 2:1?
    let (w, h) = img.dimensions();
    let is_panorama = w as f32 / h as f32 > 2.0;

    // For non-panoramas: Apply rotation, then resize
    if !(is_panorama && panorama_detection) {
        img = match orientation {
            Orientation::Deg0 => img,
            Orientation::Deg90 => img.rotate270(),
            Orientation::Deg180 => img.rotate180(),
            Orientation::Deg270 => img.rotate90(),
        }
        .resize(max_width, max_height, FilterType::CatmullRom);
    }

    // Write and return buffer
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)?;
    Ok(buf)
}

/// An image orientation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Orientation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

/// Read the orientation from the EXIF data.
///
/// In contrast to the full EXIF format, this only supports rotation, no
/// mirroring. If something goes wrong or if the image is mirrored,
/// `Orientation::Deg0` will be returned.
fn get_orientation(image_path: impl AsRef<Path>) -> Result<Orientation> {
    let file = fs::File::open(&image_path)?;
    let orientation = ExifReader::new()
        .read_from_container(&mut std::io::BufReader::new(&file))?
        .get_field(ExifTag::Orientation, IdfNum::PRIMARY)
        .map(|field| field.value.clone())
        .and_then(|val: ExifValue| {
            if let ExifValue::Short(data) = val {
                data.first().cloned()
            } else {
                None
            }
        })
        .map(|orientation| match orientation {
            1 => Orientation::Deg0,
            8 => Orientation::Deg90,
            3 => Orientation::Deg180,
            6 => Orientation::Deg270,
            _ => Orientation::Deg0,
        });
    Ok(orientation.unwrap_or(Orientation::Deg0))
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
    let mut image_files = fs::read_dir(&args.input_dir)?
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
                .map(|s| s.ends_with(".jpg") || s.ends_with(".JPG"))
                .unwrap_or(false)
        })
        .map(|dir_entry| dir_entry.path())
        .collect::<Vec<_>>();
    image_files.sort();

    // Determine download ZIP filename
    let download_filename = if args.no_download {
        None
    } else {
        let name: String = args
            .title
            .chars()
            .map(|c| if c == ' ' { '_' } else { c })
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();
        Some(format!("{}.zip", name))
    };

    // Process images
    let zipfile = Arc::new(Mutex::new(
        download_filename
            .as_ref()
            .map(|filename| fs::File::create(args.output_dir.join(filename)).unwrap())
            .map(zip::ZipWriter::new),
    ));

    let images = image_files
        .par_iter()
        .map(|f| {
            // Determine filenames
            let filename_full = f.file_name().unwrap().to_str().unwrap().to_string();
            let filename_thumb = format!(
                "{}.thumb.jpg",
                f.file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| anyhow!("Could not determine file stem for file {:?}", f))
                    .unwrap(),
            );

            // Resize
            if !args.skip_processing {
                log!("Processing {:?}", filename_full);

                // Read orientation from EXIF data
                let orientation = get_orientation(f).unwrap_or(Orientation::Deg0);

                // Generate and write thumbnail
                let thumbnail_bytes = resize_image(
                    f,
                    args.thumbnail_height * 4,
                    args.thumbnail_height,
                    &orientation,
                    false,
                )?;
                let thumbnail_path = args.output_dir.join(&filename_thumb);
                fs::write(thumbnail_path, thumbnail_bytes)?;

                // Copy original size file
                let full_path = args.output_dir.join(&filename_full);
                if let Some(max_size) = args.max_large_size {
                    let (w, h) = get_dimensions(f)?;
                    if w > max_size || h > max_size {
                        // Resize large image
                        let large_bytes = resize_image(
                            f,
                            max_size,
                            max_size,
                            &orientation,
                            !args.resize_include_panorama,
                        )?;
                        fs::write(&full_path, large_bytes)?;
                    } else {
                        // Image is smaller than max size, copy as-is
                        fs::copy(f, &full_path)?;
                    }
                } else {
                    // No max-large-size parameter specified, copy original
                    fs::copy(f, &full_path)?;
                }

                // Add file to ZIP
                let options = zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);

                if let Some(zipwriter) = zipfile.lock().expect("Couldn't lock zipfile").as_mut() {
                    zipwriter.start_file(&filename_full, options).unwrap();
                    zipwriter.write_all(&fs::read(&full_path).unwrap()).unwrap();
                }
            }

            // Store
            Ok(Image {
                filename_full,
                filename_thumb,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let download_filesize_mib = download_filename
        .as_ref()
        .map(|filename| fs::metadata(args.output_dir.join(filename)).unwrap().len())
        .map(|bytes| (bytes as f64 / 1024.0 / 1024.0).ceil() as u64);

    // Create template context
    let context = TemplateContext {
        title: args.title.clone(),
        galerio_version: VERSION,
        images,
        download_filename,
        download_filesize_mib,
        isodate: chrono::Utc::now().to_rfc3339(),
    };

    // Load templates
    let mut tera = Tera::default();
    for template in Templates::iter() {
        let template_bytes = Templates::get(&template)
            .ok_or_else(|| anyhow!("Could not load template {}", template))?
            .data;
        let template_str =
            std::str::from_utf8(&template_bytes).context("Template isn't valid UTF8")?;
        tera.add_raw_template(&template, template_str)
            .context("Could not add template")?;
    }

    // Generate index.html
    let rendered = tera.render("index.html", &tera::Context::from_serialize(&context)?)?;
    log!("Writing index.html");
    fs::write(args.output_dir.join("index.html"), rendered)?;

    // Write static files
    fs::create_dir(args.output_dir.join("static")).or_else(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    for file in StaticFiles::iter() {
        log!("Writing static file {}", file);
        let file_bytes = StaticFiles::get(&file)
            .ok_or_else(|| anyhow!("Could not load static file {}", file))?
            .data;
        fs::write(args.output_dir.join("static").join(&*file), file_bytes)?;
    }

    log!("Done!");
    Ok(())
}
