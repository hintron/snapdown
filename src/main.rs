use std::fs::{self, File};
use std::io::copy;
use std::path::Path;

use anyhow::Result;
use csv::Reader;
use rayon::prelude::*;
use ureq;

const DEFAULT_NUM_JOBS: usize = 500;

fn print_usage(program_name: &str) {
    eprintln!(
        "Usage: {} <input_csv> <output_dir> [-j <jobs>]",
        program_name
    );
    eprintln!("\nArguments:");
    eprintln!("  <input_csv>   Path to the input CSV file");
    eprintln!("  <output_dir>  Path to the output directory");
    eprintln!("\nOptions:");
    eprintln!(
        "  -j <jobs>     Number of parallel downloads (default: {})",
        DEFAULT_NUM_JOBS
    );
    eprintln!("  -h, --help    Show this help message");
}

struct Args {
    input_csv: String,
    output_dir: String,
    jobs: usize,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();

    // Check for help flag
    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help") {
        print_usage(&args[0]);
        std::process::exit(0);
    }

    // Check for required arguments
    if args.len() < 2 {
        eprintln!("Error: Missing input CSV file (first argument)\n");
        print_usage(&args[0]);
        std::process::exit(1);
    }

    if args.len() < 3 {
        eprintln!("Error: Missing output directory (second argument)\n");
        print_usage(&args[0]);
        std::process::exit(1);
    }

    // Parse optional -j flag
    let mut jobs = DEFAULT_NUM_JOBS;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "-j" {
            if i + 1 >= args.len() {
                eprintln!("Error: -j flag requires a value\n");
                print_usage(&args[0]);
                std::process::exit(1);
            }
            jobs = args[i + 1].parse().unwrap_or_else(|_| {
                eprintln!("Error: Invalid value for -j flag: {}\n", args[i + 1]);
                print_usage(&args[0]);
                std::process::exit(1);
            });
            i += 2;
        } else {
            eprintln!("Error: Unknown argument: {}\n", args[i]);
            print_usage(&args[0]);
            std::process::exit(1);
        }
    }

    Ok(Args {
        input_csv: args[1].clone(),
        output_dir: args[2].clone(),
        jobs,
    })
}

fn main() -> Result<()> {
    let args = parse_args()?;

    println!("Input CSV: {}", args.input_csv);
    println!("Output directory: {}", args.output_dir);
    println!("Parallel jobs: {}", args.jobs);

    // Configure Rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(args.jobs)
        .build_global()
        .unwrap();

    println!("Creating output directory if it doesn't exist...");
    fs::create_dir_all(&args.output_dir)?;
    println!("Reading CSV file...");
    let mut rdr = Reader::from_path(&args.input_csv)?;

    // Collect all records first
    let records: Vec<_> = rdr.records().collect::<Result<_, _>>()?;

    // Each row is of the form (timestamp_utc, format, latitude, longitude, download_url)
    records.par_iter().for_each(|row| {
        let timestamp_str = row[0].replace(' ', "_").replace(':', "-");
        let format = &row[1];
        let latitude = &row[2];
        let longitude = &row[3];
        let download_url = &row[4];

        let ext = match format {
            "Image" => "jpg",
            // "Image" => "png",
            "Video" => "mp4",
            _ => "bin",
        };

        let filename = format!("{}_{}_{}.{}", timestamp_str, latitude, longitude, ext);
        let path = Path::new(&args.output_dir).join(filename);

        if path.exists() {
            println!("File already exists; skipping download: {:?}", path);
            return;
        }

        println!("Creating file at path: {:?}", path);
        let mut file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Error creating file {:?}: {}", path, e);
                return;
            }
        };
        println!("Downloading from URL: {}", download_url);
        let mut resp = match ureq::get(download_url).call() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error downloading from {}: {}", download_url, e);
                return;
            }
        };
        println!("Writing to file: {:?}", file);
        if let Err(e) = copy(&mut resp.body_mut().as_reader(), &mut file) {
            eprintln!("Error writing to file {:?}: {}", path, e);
        }

        // println!("Saved {:?}", path);
    });

    Ok(())
}
