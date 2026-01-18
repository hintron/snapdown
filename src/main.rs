#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::fs::{self, File};
use std::io::copy;
use std::path::Path;

use anyhow::Result;
use csv::Reader;
use eframe::egui;
use rayon::prelude::*;
use ureq;

#[derive(Default)]
struct SnapdownEframeApp {
    dropped_files: Vec<egui::DroppedFile>,
    picked_path: Option<String>,
}

impl eframe::App for SnapdownEframeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Drag-and-drop the .csv file onto the window! Or, click the button below to pick a file.");

            if ui.button("Open file…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_file()
            {
                self.picked_path = Some(path.display().to_string());
            }

            if let Some(picked_path) = &self.picked_path {
                ui.horizontal(|ui| {
                    ui.label("Picked file:");
                    ui.monospace(picked_path);
                });
            }

            // Show dropped files (if any):
            if !self.dropped_files.is_empty() {
                ui.group(|ui| {
                    ui.label("Dropped files:");

                    for file in &self.dropped_files {
                        let mut info = if let Some(path) = &file.path {
                            path.display().to_string()
                        } else if !file.name.is_empty() {
                            file.name.clone()
                        } else {
                            "???".to_owned()
                        };

                        let mut additional_info = vec![];
                        if !file.mime.is_empty() {
                            additional_info.push(format!("type: {}", file.mime));
                        }
                        if let Some(bytes) = &file.bytes {
                            additional_info.push(format!("{} bytes", bytes.len()));
                        }
                        if !additional_info.is_empty() {
                            info += &format!(" ({})", additional_info.join(", "));
                        }

                        ui.label(info);
                    }
                });
            }

            match &self.picked_path {
                Some(picked_path) => {
                    ui.label(format!("Using picked file: {}", picked_path));
                    if ui.button("Run SnapDown").clicked()
                    {
                        match run_downloader(picked_path, "snapdown_output", DEFAULT_NUM_JOBS) {
                            Ok(_) => println!("SnapDown completed successfully."),
                            Err(e) => eprintln!("Error running SnapDown: {}", e),
                        };
                    }
                },
                None => {}
            }
        });

        preview_files_being_dropped(ctx);

        // Collect dropped files:
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                self.dropped_files.clone_from(&i.raw.dropped_files);
            }
        });
    }
}

/// Preview hovering files:
fn preview_files_being_dropped(ctx: &egui::Context) {
    use egui::{Align2, Color32, Id, LayerId, Order, TextStyle};
    use std::fmt::Write as _;

    if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
        let text = ctx.input(|i| {
            let mut text = "Dropping files:\n".to_owned();
            for file in &i.raw.hovered_files {
                if let Some(path) = &file.path {
                    write!(text, "\n{}", path.display()).ok();
                } else if !file.mime.is_empty() {
                    write!(text, "\n{}", file.mime).ok();
                } else {
                    text += "\n???";
                }
            }
            text
        });

        let painter =
            ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("file_drop_target")));

        let content_rect = ctx.content_rect();
        painter.rect_filled(content_rect, 0.0, Color32::from_black_alpha(192));
        painter.text(
            content_rect.center(),
            Align2::CENTER_CENTER,
            text,
            TextStyle::Heading.resolve(&ctx.style()),
            Color32::WHITE,
        );
    }
}

const DEFAULT_NUM_JOBS: usize = 500;

fn print_usage(program_name: &str) {
    eprintln!(
        "Usage: {} [--cli -i <input_csv> -o <output_dir> -j <jobs>]",
        program_name
    );
    eprintln!("\nOptions:");
    eprintln!("  --cli     Use the command line interface instead of the GUI, with options below:");
    eprintln!("  -i <input_csv>   Path to the input CSV file");
    eprintln!("  -o <output_dir>  Path to the output directory");
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
    cli: bool,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();

    // Check for help flag
    if args.len() > 1 && (args[1] == "-h" || args[1] == "--help") {
        print_usage(&args[0]);
        std::process::exit(0);
    }

    let mut input_csv = None;
    let mut output_dir = None;
    let mut jobs = DEFAULT_NUM_JOBS;
    let mut cli = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: -i flag requires a value\n");
                    print_usage(&args[0]);
                    std::process::exit(1);
                }
                input_csv = Some(args[i + 1].clone());
                i += 2;
            }
            "-o" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: -o flag requires a value\n");
                    print_usage(&args[0]);
                    std::process::exit(1);
                }
                output_dir = Some(args[i + 1].clone());
                i += 2;
            }
            "-j" => {
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
            }
            "--cli" => {
                cli = true;
                i += 1;
            }
            _ => {
                eprintln!("Error: Unknown argument: {}\n", args[i]);
                print_usage(&args[0]);
                std::process::exit(1);
            }
        }
    }

    // Only require -i and -o if CLI mode is enabled
    if cli {
        let input_csv = input_csv.ok_or_else(|| {
            eprintln!("Error: Missing required argument -i <input_csv>\n");
            print_usage(&args[0]);
            std::process::exit(1);
        })?;

        let output_dir = output_dir.ok_or_else(|| {
            eprintln!("Error: Missing required argument -o <output_dir>\n");
            print_usage(&args[0]);
            std::process::exit(1);
        })?;

        Ok(Args {
            input_csv,
            output_dir,
            jobs,
            cli,
        })
    } else {
        Ok(Args {
            input_csv: input_csv.unwrap_or_default(),
            output_dir: output_dir.unwrap_or_default(),
            jobs,
            cli,
        })
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;

    println!("Input CSV: {}", args.input_csv);
    println!("Output directory: {}", args.output_dir);
    println!("Parallel jobs: {}", args.jobs);

    if args.cli {
        return run_downloader(&args.input_csv, &args.output_dir, args.jobs);
    } else {
        return run_gui();
    }
}

fn run_gui() -> Result<()> {
    // Have the GUI take care of getting args from the user
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 240.0]) // wide enough for the drag-drop overlay text
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "SnapDown GUI",
        options,
        Box::new(|_cc| Ok(Box::<SnapdownEframeApp>::default())),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run GUI: {}", e))
}

fn run_downloader(input_csv: &str, output_dir: &str, jobs: usize) -> Result<()> {
    // Configure Rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build_global()
        .unwrap();

    println!("Creating output directory if it doesn't exist...");
    fs::create_dir_all(output_dir)?;
    println!("Reading CSV file...");
    let mut rdr = Reader::from_path(input_csv)?;

    // Collect all records first
    let records: Vec<_> = rdr.records().collect::<Result<_, _>>()?;

    println!("Downloading {} files:", records.len());
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
            "PNG" => "png",
            "SVG" => "svg",
            _ => "bin",
        };

        let filename = format!("{}_{}_{}.{}", timestamp_str, latitude, longitude, ext);
        let path = Path::new(output_dir).join(filename);

        if path.exists() {
            println!("  * File already exists; skipping download: {:?}", path);
            return;
        }

        let mut file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("  * Error creating file {:?}: {}", path, e);
                return;
            }
        };

        let mut resp = match ureq::get(download_url).call() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  * Error downloading from {}: {}", download_url, e);
                return;
            }
        };

        if let Err(e) = copy(&mut resp.body_mut().as_reader(), &mut file) {
            eprintln!(
                "  * Downloaded, but error writing to file {:?}: {}",
                path, e
            );
        }
        println!("  * Downloaded {}", download_url);
    });

    Ok(())
}
