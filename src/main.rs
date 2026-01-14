// use std::fs::{self, File};
use std::fs::{self};
use std::path::Path;

use anyhow::Result;
use csv::Reader;
// use ureq;

fn print_usage(program_name: &str) {
    eprintln!("Usage: {} <input_csv> <output_dir>", program_name);
    eprintln!("\nArguments:");
    eprintln!("  <input_csv>   Path to the input CSV file");
    eprintln!("  <output_dir>  Path to the output directory");
    eprintln!("\nOptions:");
    eprintln!("  -h, --help    Show this help message");
}

struct Args {
    input_csv: String,
    output_dir: String,
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

    Ok(Args {
        input_csv: args[1].clone(),
        output_dir: args[2].clone(),
    })
}

fn main() -> Result<()> {
    let args = parse_args()?;

    println!("Input CSV: {}", args.input_csv);
    println!("Output directory: {}", args.output_dir);

    println!("Creating output directory if it doesn't exist...");
    fs::create_dir_all(args.output_dir)?;
    println!("Reading CSV file...");
    let mut rdr = Reader::from_path(args.input_csv)?;

    // Each row is of the form (timestamp_utc, format, latitude, longitude, download_url)
    for result in rdr.records() {
        let row = result?;
        println!("{:?}", row);
        let timestamp_str = row[0].replace(' ', "_");
        // let dt: DateTime<Utc> =
        //     DateTime::parse_from_str(&row[0], "%Y-%m-%d %H:%M:%S UTC")?
        //         .with_timezone(&Utc);

        let ext = match &row[1] {
            "Image" => "jpg",
            // "Image" => "png",
            "Video" => "mp4",
            _ => "bin",
        };

        let filename = format!("{}_{}_{}.{}", timestamp_str, &row[2], &row[3], ext);

        let path = Path::new("out").join(filename);

        if path.exists() {
            println!("File already exists; skipping download: {:?}", path);
            continue;
        }

        // let resp = ureq::get(&row.download_url).call()?;
        // let mut file = File::create(&path)?;
        // copy(&mut resp.into_reader(), &mut file)?;

        // set_file_timestamp(&path, dt)?;
        // set_exif_metadata(&path, dt)?;

        // println!("Saved {:?}", path);
    }

    Ok(())
}
