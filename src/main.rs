#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::fs::{self, File};
use std::io::copy;
use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use circular_buffer::CircularBuffer;
use csv::Reader;
use eframe::egui;
use rayon::prelude::*;
use ureq;

struct SnapdownStatus {
    finished: bool,
    error_count: usize,
    success_count: usize,
    skip_count: usize,
}

enum SnapdownState {
    Idle,
    SelectingFile,
    Downloading,
    Completed,
    // Error,
}

struct SnapdownEframeApp {
    picked_path: Option<String>,
    state: SnapdownState,
    recv_from_filepicker: mpsc::Receiver<String>,
    send_from_filepicker: mpsc::Sender<String>,
    recv_logs_from_downloader: mpsc::Receiver<String>,
    send_logs_from_downloader: mpsc::Sender<String>,
    recv_status_from_downloader: mpsc::Receiver<SnapdownStatus>,
    send_status_from_downloader: mpsc::Sender<SnapdownStatus>,
    success_count: usize,
    error_count: usize,
    skip_count: usize,
    // This will act as a circular buffer to limit memory usage
    messages_console: CircularBuffer<1024, String>,
}

impl eframe::App for SnapdownEframeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ////////////////////////////////////////////////////////////////////
            // Header/Control Section
            ////////////////////////////////////////////////////////////////////
            ui.heading("SnapDown: Download SnapChat files quickly!");

            if ui.button("Open .csv file...").clicked() {
                // Open file dialog in separate thread to avoid blocking UI
                // Clone the sender for use in the thread
                let send_from_filepicker_clone = self.send_from_filepicker.clone();
                std::thread::spawn(move || {
                    match rfd::FileDialog::new().pick_file() {
                        Some(path) => {
                            // Once file is picked, send it back to the UI thread
                            match send_from_filepicker_clone.send(path.display().to_string()) {
                                Err(e) => {
                                    eprintln!("Error sending picked file path to UI thread: {}", e);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                });
                self.state = SnapdownState::SelectingFile;
            }

            self.recv_from_filepicker
                .try_iter()
                .for_each(|picked_path| {
                    println!(
                        "Picked file and received it from picker thread: {}",
                        picked_path
                    );
                    self.picked_path = Some(picked_path);
                    self.state = SnapdownState::Idle;
                });

            match &self.picked_path {
                Some(picked_path) => {
                    ui.horizontal(|ui| {
                        ui.label("Picked file:");
                        ui.monospace(picked_path);
                    });

                    if ui.button("Run SnapDown").clicked() {
                        let picked_path = picked_path.clone();
                        let send_logs_from_downloader_clone =
                            self.send_logs_from_downloader.clone();
                        let send_status_from_downloader_clone =
                            self.send_status_from_downloader.clone();
                        std::thread::spawn(move || {
                            match run_downloader(
                                &picked_path,
                                "snapdown_output",
                                DEFAULT_NUM_JOBS,
                                Some(send_logs_from_downloader_clone),
                                Some(send_status_from_downloader_clone),
                            ) {
                                Ok(_) => println!("SnapDown completed successfully."),
                                Err(e) => eprintln!("Error running SnapDown: {}", e),
                            }
                        });
                        self.state = SnapdownState::Downloading;
                    }
                }
                None => {}
            }

            self.recv_status_from_downloader
                .try_iter()
                .for_each(|status| {
                    if status.finished {
                        self.state = SnapdownState::Completed;
                    } else {
                        self.state = SnapdownState::Downloading;
                    }
                    self.success_count = status.success_count;
                    self.error_count = status.error_count;
                    self.skip_count = status.skip_count;
                });

            ui.separator();
            ui.heading("Status");
            ui.separator();
            match self.state {
                SnapdownState::Idle => {
                    ui.label("Idle. Ready to start downloading.");
                }
                SnapdownState::SelectingFile => {
                    ui.label("Selecting file...");
                }
                SnapdownState::Downloading => {
                    ui.label("Downloading files...");
                    ui.label(format!("Successful downloads: {}", self.success_count));
                    ui.label(format!("Errors: {}", self.error_count));
                    ui.label(format!("Skipped: {}", self.skip_count));
                }
                SnapdownState::Completed => {
                    ui.label("Download completed!");
                    ui.label(format!("Successful downloads: {}", self.success_count));
                    ui.label(format!("Errors: {}", self.error_count));
                    ui.label(format!("Skipped: {}", self.skip_count));
                }
            }
            ui.heading("Console Log (last 1024 messages)");
            ui.separator();
            ////////////////////////////////////////////////////////////////////
            // Console Log Section
            ////////////////////////////////////////////////////////////////////
            self.recv_logs_from_downloader.try_iter().for_each(|msg| {
                self.messages_console.push_back(msg);
            });

            // Capture remaining space
            let available = ui.available_size();

            // ----- scrollable content -----
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_size(available);

                    for message in &self.messages_console {
                        ui.monospace(message);
                    }
                });
        });
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

    if args.cli {
        println!("Input CSV: {}", args.input_csv);
        println!("Output directory: {}", args.output_dir);
        println!("Parallel jobs: {}", args.jobs);
        return run_downloader(&args.input_csv, &args.output_dir, args.jobs, None, None);
    } else {
        return run_gui();
    }
}

fn run_gui() -> Result<()> {
    let (send_from_filepicker, recv_from_filepicker) = mpsc::channel::<String>();
    let (send_logs_from_downloader, recv_logs_from_downloader) = mpsc::channel::<String>();
    let (send_status_from_downloader, recv_status_from_downloader) =
        mpsc::channel::<SnapdownStatus>();
    let snapdown_app = SnapdownEframeApp {
        picked_path: None,
        state: SnapdownState::Idle,
        send_from_filepicker: send_from_filepicker,
        recv_from_filepicker: recv_from_filepicker,
        send_logs_from_downloader: send_logs_from_downloader,
        recv_logs_from_downloader: recv_logs_from_downloader,
        send_status_from_downloader: send_status_from_downloader,
        recv_status_from_downloader: recv_status_from_downloader,
        success_count: 0,
        error_count: 0,
        skip_count: 0,
        messages_console: CircularBuffer::<1024, String>::new(),
    };

    // Have the GUI take care of getting args from the user
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SnapDown GUI",
        options,
        Box::new(|_cc| Ok(Box::new(snapdown_app))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run GUI: {}", e))
}

fn log_message(gui_console: &Option<mpsc::Sender<String>>, message: String) {
    match gui_console {
        Some(sender) => {
            sender.send(message).unwrap_or_else(|e| {
                eprintln!("Error sending message to GUI console: {}", e);
            });
        }
        None => {
            println!("{}", message);
        }
    }
}

fn log_error(gui_console: &Option<mpsc::Sender<String>>, message: String) {
    match gui_console {
        Some(sender) => {
            sender.send(message).unwrap_or_else(|e| {
                eprintln!("Error sending message to GUI console: {}", e);
            });
        }
        None => {
            eprintln!("{}", message);
        }
    }
}

fn run_downloader(
    input_csv: &str,
    output_dir: &str,
    jobs: usize,
    gui_console: Option<mpsc::Sender<String>>,
    status_sender: Option<mpsc::Sender<SnapdownStatus>>,
) -> Result<()> {
    // Configure Rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build_global()
        .unwrap();

    log_message(
        &gui_console,
        "Creating output directory if it doesn't exist...".to_string(),
    );

    fs::create_dir_all(output_dir)?;
    log_message(&gui_console, "Reading CSV file...".to_string());
    let mut rdr = Reader::from_path(input_csv)?;

    // Collect all records first
    let records: Vec<_> = rdr.records().collect::<Result<_, _>>()?;

    log_message(
        &gui_console,
        format!("Downloading {} files:", records.len()),
    );

    let success_count = std::sync::atomic::AtomicUsize::new(0);
    let error_count = std::sync::atomic::AtomicUsize::new(0);
    let skip_count = std::sync::atomic::AtomicUsize::new(0);
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
            log_message(
                &gui_console,
                format!("  * File already exists; skipping download: {:?}", path),
            );
            skip_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        let mut resp = match ureq::get(download_url).call() {
            Ok(r) => r,
            Err(e) => {
                log_error(
                    &gui_console,
                    format!("  * Error downloading from {}: {}", download_url, e),
                );
                error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };

        // Create the file AFTER the download, so we don't have a ton of open
        // files and exhaust Linux's default per-process open file limit.
        let mut file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                log_error(
                    &gui_console,
                    format!("  * Error creating file {:?}: {}", path, e),
                );
                error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };

        match copy(&mut resp.body_mut().as_reader(), &mut file) {
            Ok(_) => {
                log_message(&gui_console, format!("  * Downloaded {}", download_url));
                success_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                log_error(
                    &gui_console,
                    format!(
                        "  * Downloaded, but error writing to file {:?}: {}",
                        path, e
                    ),
                );
                error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        // Every 10 items send a status update
        match &status_sender {
            Some(sender) => {
                let total_success = success_count.load(std::sync::atomic::Ordering::Relaxed);
                let total_error = error_count.load(std::sync::atomic::Ordering::Relaxed);
                let total_skip = skip_count.load(std::sync::atomic::Ordering::Relaxed);
                let status = SnapdownStatus {
                    finished: false,
                    success_count: total_success,
                    error_count: total_error,
                    skip_count: total_skip,
                };
                sender.send(status).unwrap_or_else(|e| {
                    eprintln!("Error sending status to GUI: {}", e);
                });
            }
            None => {}
        }
    });

    let success_count = success_count.load(std::sync::atomic::Ordering::Relaxed);
    let error_count = error_count.load(std::sync::atomic::Ordering::Relaxed);
    let skip_count = skip_count.load(std::sync::atomic::Ordering::Relaxed);

    match &status_sender {
        Some(sender) => {
            let status = SnapdownStatus {
                finished: true,
                success_count: success_count,
                error_count: error_count,
                skip_count: skip_count,
            };
            sender.send(status).unwrap_or_else(|e| {
                eprintln!("Error sending status to GUI: {}", e);
            });
        }
        None => {}
    }

    log_message(
        &gui_console,
        format!("Finished processing {} links", records.len()),
    );
    if success_count > 0 {
        log_message(
            &gui_console,
            format!("  - Success: {} files", records.len()),
        );
    }
    if error_count > 0 {
        log_error(&gui_console, format!("  - Error: {} files", error_count));
    }
    if skip_count > 0 {
        log_message(
            &gui_console,
            format!("  - Skipped: {} files (already existed)", skip_count),
        );
    }
    log_message(&gui_console, format!("SnapDown completed"));

    Ok(())
}
