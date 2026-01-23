#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use std::fs::{self, File};
use std::io::{BufRead, BufReader, copy};
use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use chrono;
use circular_buffer::CircularBuffer;
use csv::Reader;
use eframe::egui;
use egui::{Color32, FontId, TextStyle};
use env_logger::{Builder, Env};
use log::{debug, error, info};
use rayon::prelude::*;
use std::fs::OpenOptions;
use std::io::Write;
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
    // Flag to ensure style is only on the first update, then saved to context
    style_applied: bool,
}

impl eframe::App for SnapdownEframeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set up custom styling (do this only once)
        if !self.style_applied {
            let mut style = (*ctx.style()).clone();

            style.visuals.window_fill = Color32::YELLOW;
            style.visuals.panel_fill = Color32::YELLOW;
            style.visuals.extreme_bg_color = Color32::WHITE;
            // style.visuals.override_text_color = Some(Color32::BLACK);

            style.visuals.window_corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);

            style.spacing.button_padding = egui::vec2(12.0, 8.0);
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);

            style
                .text_styles
                .insert(TextStyle::Heading, FontId::proportional(24.0));
            style
                .text_styles
                .insert(TextStyle::Body, FontId::proportional(16.0));
            style
                .text_styles
                .insert(TextStyle::Button, FontId::proportional(16.0));

            ctx.set_style(style);
            self.style_applied = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ////////////////////////////////////////////////////////////////////
            // Header/Control Section
            ////////////////////////////////////////////////////////////////////
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.heading("SnapDown: Download SnapChat files quickly!");

                if ui
                    .button("Open memories_history.html or snap_export.csv file...")
                    .clicked()
                {
                    // Open file dialog in separate thread to avoid blocking UI
                    // Clone the sender for use in the thread
                    let send_from_filepicker_clone = self.send_from_filepicker.clone();
                    std::thread::spawn(move || {
                        match rfd::FileDialog::new().pick_file() {
                            Some(path) => {
                                // Once file is picked, send it back to the UI thread
                                match send_from_filepicker_clone.send(path.display().to_string()) {
                                    Err(e) => {
                                        error!(
                                            "Error sending picked file path to UI thread: {}",
                                            e
                                        );
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    });
                    self.state = SnapdownState::SelectingFile;
                }
            });

            self.recv_from_filepicker
                .try_iter()
                .for_each(|picked_path| {
                    info!(
                        "Picked file and received it from picker thread: {}",
                        picked_path
                    );
                    self.picked_path = Some(picked_path);
                    self.state = SnapdownState::Idle;
                });

            match &self.picked_path {
                Some(picked_path) => {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        ui.label("Picked file:");
                        ui.monospace(picked_path);

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
                                    Some(&send_logs_from_downloader_clone),
                                    Some(&send_status_from_downloader_clone),
                                ) {
                                    Ok(_) => log_message(
                                        Some(&send_logs_from_downloader_clone),
                                        "SnapDown completed successfully.".to_string(),
                                    ),
                                    Err(e) => log_error(
                                        Some(&send_logs_from_downloader_clone),
                                        format!("Error running SnapDown: {}", e),
                                    ),
                                }
                            });
                            self.state = SnapdownState::Downloading;
                        }
                    });
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
            ui.heading("Console Log (last 1024 messages only; see snapdown.log for full log)");
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

fn init_logging() {
    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open("snapdown.log")
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error opening log file snapdown.log: {}", e);
            std::process::exit(1);
        }
    };

    // Set all dependencies to log at error, and all snapdown logs to info
    // Pipe the output to the log file
    Builder::from_env(Env::new().filter_or("SNAPDOWN_LOG", "error,snapdown=info"))
        .target(env_logger::Target::Pipe(Box::new(file)))
        .format(move |buf, record| {
            writeln!(
                buf,
                "[{}][{}] {}",
                record.level(),
                record.target(),
                record.args()
            )
        })
        .init();
}

fn main() -> Result<()> {
    let args = parse_args()?;

    init_logging();

    if args.cli {
        info!(
            "[{}] Starting SnapDown (CLI mode)...",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        info!("Input CSV: {}", args.input_csv);
        info!("Output directory: {}", args.output_dir);
        info!("Parallel jobs: {}", args.jobs);
        return run_downloader(&args.input_csv, &args.output_dir, args.jobs, None, None);
    } else {
        info!(
            "[{}] Starting SnapDown (GUI mode)...",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
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
        style_applied: false,
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

fn log_message(gui_console: Option<&mpsc::Sender<String>>, message: String) {
    info!("{}", &message);
    match gui_console {
        Some(sender) => {
            sender.send(message).unwrap_or_else(|e| {
                error!("Error sending message to GUI console: {}", e);
            });
        }
        None => {}
    }
}

fn log_error(gui_console: Option<&mpsc::Sender<String>>, message: String) {
    error!("{}", &message);
    match gui_console {
        Some(sender) => {
            sender.send(message).unwrap_or_else(|e| {
                error!("Error sending message to GUI console: {}", e);
            });
        }
        None => {}
    }
}

// // Helper function to find a pattern in bytes, returns position if found
// fn find_pattern(haystack: &[u8], needle: &[u8]) -> Option<usize> {
//     if needle.is_empty() || haystack.len() < needle.len() {
//         return None;
//     }

//     for i in 0..=(haystack.len() - needle.len()) {
//         if &haystack[i..i + needle.len()] == needle {
//             return Some(i);
//         }
//     }
//     None
// }

// // Extract latitude and longitude from location string
// fn extract_coordinates(location: &str) -> (Option<String>, Option<String>) {
//     // Look for pattern like "Latitude, Longitude: 40.25548, -111.645325"
//     if let Some(colon_pos) = location.find(':') {
//         let coords_part = &location[colon_pos + 1..].trim();
//         let parts: Vec<&str> = coords_part.split(',').collect();
//         if parts.len() >= 2 {
//             let lat = parts[0].trim().to_string();
//             let lng = parts[1].trim().to_string();
//             return (Some(lat), Some(lng));
//         }
//     }
//     (None, None)
// }

// // Extract download URL from onclick attribute
// fn extract_download_url(td_content: &[u8]) -> Option<String> {
//     let content = String::from_utf8_lossy(td_content);

//     // Look for downloadMemories('URL' pattern
//     if let Some(start) = content.find("downloadMemories('") {
//         let start_pos = start + 18; // Length of "downloadMemories('"
//         if let Some(end) = content[start_pos..].find("'") {
//             return Some(content[start_pos..start_pos + end].to_string());
//         }
//     }
//     None
// }

// Enum to represent the search result
#[derive(Debug)]
enum SearchResult {
    NotFound,
    Found(usize),                   // Index where found
    NotFoundWithUnprocessed(usize), // Number of unprocessed bytes at the end
}

// Linearly look for a pattern of bytes in a buffer. If found, return the
// index where the tag was found in that buffer.
// If is_last is true, then it means that this is the end of the data and we
// don't need to combine the end of this buffer with the beginning of the next
// buffer.
fn look_for_item(buffer: &[u8], item: &[u8], is_last: bool) -> SearchResult {
    let item_size = item.len();
    let buffer_size = buffer.len();

    if buffer_size <= 0 {
        // Empty buffer
        return SearchResult::NotFound;
    }
    if buffer_size < item_size {
        // The buffer is too small to possibly contain the item
        if is_last {
            return SearchResult::NotFound;
        } else {
            return SearchResult::NotFoundWithUnprocessed(buffer_size);
        }
    }
    assert!(item_size > 0, "Item size must be greater than zero");

    for (index, window) in buffer.windows(item_size).enumerate() {
        // info!(
        //     "{}: {} vs. {}",
        //     index,
        //     String::from_utf8_lossy(window),
        //     String::from_utf8_lossy(item)
        // );
        if window == item {
            return SearchResult::Found(index);
        }
    }

    // We did not find the item

    // This is the last buffer, so the windows covered all bytes
    if is_last {
        return SearchResult::NotFound;
    }

    // The end of this buffer needs to be combined with the start of the next
    // buffer, and windows() can't check the last (item_size - 1) bytes
    let unprocessed = item_size - 1;
    SearchResult::NotFoundWithUnprocessed(unprocessed)
}

#[derive(Debug)]
enum SdParseState {
    SearchingForTable,
    SearchingForTbody,
    SearchingForTr,
    SearchingForTh,
    SearchingForThEnd,
    SearchingForThClosing,
    SearchingForTd,
    SearchingForTdEnd,
    SearchingForTdClosing,
    SearchingForDownloadLink,
    SearchingForDownloadLinkEnd,
    // SearchingForTrClosing,
    // SearchingForTableClosing,
    // SearchingForTbodyClosing,
    // SearchingForHtmlTagEnd,
    // SearchingForHtmlTagStart,
    // SearchingForNextNonWhitespace,
    // SearchingForAttribute,
    // SearchingForAttributeEnd,
    // SearchingForAttributeValueStart,
    // SearchingForAttributeValueEnd,
    // SearchingForQuote,
    // SearchingForQuoteEnd,
    // LookingForDate,
    // LookingForMediaType,
    // LookingForLocation,
    // LookingForDownloadLink,
}

// fn parse_next(buffer: &[u8], state: &SdParseState) -> usize {
//     return 0;
// }

fn parse_memories_history_html(
    input_file: &str,
    gui_console: Option<&mpsc::Sender<String>>,
) -> Result<Vec<csv::StringRecord>> {
    log_message(
        gui_console,
        "Detected HTML file (memories_history.html). Converting to CSV format...".to_string(),
    );

    // Read HTML file and convert to CSV format
    let html_file = File::open(input_file)?;
    const BUFFER_SIZE: usize = 1024 * 16;
    let mut html_reader = BufReader::with_capacity(BUFFER_SIZE, html_file);

    let mut csv_records: Vec<csv::StringRecord> = Vec::new();
    let mut file_byte_index = 0u64;
    let mut parse_state = SdParseState::SearchingForTable;
    let mut header_column_count = 0usize;
    let mut row_column_count = 0usize;
    let mut current_record = csv::StringRecord::new();
    const EXPECTED_COLUMNS: usize = 4;

    loop {
        // Parsing logic
        // For an example of the HTML data we want to parse, see test_parse_html_snippet()

        // Determine if there is anything we need to grab before looking for the
        // next tag, and set what tag to look for next
        let tag = match parse_state {
            SdParseState::SearchingForTable => Some("<table>"),
            SdParseState::SearchingForTbody => Some("<tbody>"),
            SdParseState::SearchingForTr => Some("<tr>"),
            SdParseState::SearchingForTh => Some("<th"),
            SdParseState::SearchingForThEnd => Some(">"),
            SdParseState::SearchingForThClosing => Some("</th>"),
            SdParseState::SearchingForTd => Some("<td"),
            SdParseState::SearchingForTdEnd => Some(">"),
            SdParseState::SearchingForTdClosing => Some("</td>"),
            SdParseState::SearchingForDownloadLink => Some("downloadMemories('"),
            SdParseState::SearchingForDownloadLinkEnd => Some("',"),
            // SdParseState::SearchingForTrClosing => Some("</tr>"),
            // SdParseState::SearchingForHtmlTagEnd => Some(">"),
            // _ => None,
        };

        match tag {
            Some(tag) => {
                // Since we are looking for a tag, read in data and search for it
                let buffer = html_reader.fill_buf()?;
                if buffer.is_empty() {
                    break; // EOF
                }

                let is_last = buffer.len() < BUFFER_SIZE;
                log_message(
                    gui_console,
                    format!(
                        "File byte index {}: Parsing {} bytes for tag '{}'... (is_last={})",
                        file_byte_index,
                        buffer.len(),
                        tag,
                        is_last
                    ),
                );
                let processed;
                match look_for_item(&buffer, tag.as_bytes(), is_last) {
                    SearchResult::Found(index) => {
                        info!(
                            "Found '{}' at file byte index {} (buffer byte index {index})",
                            tag,
                            file_byte_index + (index as u64)
                        );
                        processed = index + tag.len();

                        // Move on to next tag
                        parse_state = match parse_state {
                            SdParseState::SearchingForTable => SdParseState::SearchingForTbody,
                            SdParseState::SearchingForTbody => SdParseState::SearchingForTr,
                            SdParseState::SearchingForTr => {
                                if header_column_count == 0 {
                                    SdParseState::SearchingForTh
                                } else {
                                    SdParseState::SearchingForTd
                                }
                            }
                            SdParseState::SearchingForTh => SdParseState::SearchingForThEnd,
                            SdParseState::SearchingForThEnd => SdParseState::SearchingForThClosing,
                            SdParseState::SearchingForThClosing => {
                                current_record
                                    .push_field(&String::from_utf8_lossy(&buffer[..index]).trim());
                                header_column_count += 1;
                                if header_column_count >= EXPECTED_COLUMNS {
                                    // Validate that the data looks as expected


                                    // Finished header row
                                    csv_records.push(current_record.clone());
                                    // Reset for data row
                                    current_record.clear();
                                    SdParseState::SearchingForTr
                                } else {
                                    // Keep looking for header columns
                                    SdParseState::SearchingForTh
                                }
                            }
                            SdParseState::SearchingForTd => SdParseState::SearchingForTdEnd,
                            SdParseState::SearchingForTdEnd => {
                                if row_column_count == 3 {
                                    // Look for the download link inside this td
                                    SdParseState::SearchingForDownloadLink
                                } else {
                                    // Generic td content - save it all
                                    SdParseState::SearchingForTdClosing
                                }
                            }
                            SdParseState::SearchingForTdClosing => {
                                current_record
                                    .push_field(&String::from_utf8_lossy(&buffer[..index]).trim());
                                row_column_count += 1;
                                if row_column_count == 3 {
                                    // Parse the last column, the download link
                                    SdParseState::SearchingForDownloadLink
                                } else {
                                    // Keep looking for more row data columns
                                    SdParseState::SearchingForTd
                                }
                            }
                            // SdParseState::SearchingForTrClosing => SdParseState::SearchingForTr,
                            SdParseState::SearchingForDownloadLink => {
                                SdParseState::SearchingForDownloadLinkEnd
                            }
                            SdParseState::SearchingForDownloadLinkEnd => {
                                // This should be the last column in the row
                                if row_column_count + 1 != EXPECTED_COLUMNS {
                                    log_error(
                                        gui_console,
                                        format!(
                                            "Row {} had an unexpected number of columns",
                                            row_column_count
                                        ),
                                    );
                                }
                                current_record
                                    .push_field(&String::from_utf8_lossy(&buffer[..index]).trim());
                                csv_records.push(current_record.clone());
                                // Reset for next data row
                                current_record.clear();
                                row_column_count = 0;
                                // Skip looking for td end, since we got what we
                                // wanted. Move on to next data row
                                SdParseState::SearchingForTr
                            } // state => unimplemented!("Unhandled parse state: {:?}", state),
                        }
                    }
                    SearchResult::NotFoundWithUnprocessed(n) => processed = buffer.len() - n,
                    SearchResult::NotFound => processed = buffer.len(),
                }

                // Parsing progress has been made; advance internal cursor
                html_reader.consume(processed);
                file_byte_index += processed as u64;
            }
            None => {}
        }
    }

    info!("Finished reading HTML file.");
    Ok(csv_records)
}

fn run_downloader(
    input_file: &str,
    output_dir: &str,
    jobs: usize,
    gui_console: Option<&mpsc::Sender<String>>,
    status_sender: Option<&mpsc::Sender<SnapdownStatus>>,
) -> Result<()> {
    // Configure Rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build_global()
        .unwrap();

    log_message(
        gui_console,
        "Creating output directory if it doesn't exist...".to_string(),
    );

    fs::create_dir_all(output_dir)?;
    log_message(gui_console, format!("Reading input file {input_file}..."));

    let records: Vec<_>;
    // Determine if this is memories_history.html or snap_export.csv
    if input_file.ends_with("memories_history.html") {
        records = parse_memories_history_html(input_file, gui_console)?;
    } else if input_file.ends_with("snap_export.csv") {
        log_message(
            gui_console,
            "Detected CSV file (snap_export.html). Extracting records...".to_string(),
        );

        let mut rdr = Reader::from_path(input_file)?;

        // Collect all records first
        records = rdr.records().collect::<Result<_, _>>()?;
    } else {
        log_error(
            gui_console,
            "Input file is neither memories_history.html nor snap_export.csv format. Exiting."
                .to_string(),
        );
        return Err(anyhow::anyhow!(
            "Input file is neither memories_history.html nor snap_export.csv format. Exiting."
        ));
    }

    log_message(gui_console, format!("Downloading {} files:", records.len()));

    let success_count = std::sync::atomic::AtomicUsize::new(0);
    let error_count = std::sync::atomic::AtomicUsize::new(0);
    let skip_count = std::sync::atomic::AtomicUsize::new(0);
    // Each row is of the form (timestamp_utc, format, latitude, longitude, download_url)
    records.par_iter().for_each(|row| {
        let row_len = row.len();
        if row_len == 0 {
            // Skip empty rows
            log_error(gui_console, format!("Row was empty. Skipping download"));
            error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        if row_len < 4 || row_len > 5 {
            // Bad row data
            log_error(
                gui_console,
                format!(
                    "Row had unexpected number of columns ({}). Skipping download",
                    row_len
                ),
            );
            error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        assert!((row_len == 4) || (row_len == 5));

        let timestamp_str = row[0].replace(' ', "_").replace(':', "-");
        let format = &row[1];
        let ext = match format {
            "Image" => "jpg",
            // "Image" => "png",
            "Video" => "mp4",
            "PNG" => "png",
            "SVG" => "svg",
            _ => "bin",
        };

        let (filename, download_url) = if row_len == 5 {
            // Assume timestamp, format, latitude, longitude, download_url
            let latitude = &row[2];
            let longitude = &row[3];
            let download_url = &row[4];
            (
                format!("{}_{}_{}.{}", timestamp_str, latitude, longitude, ext),
                download_url,
            )
        } else {
            // Assume timestamp, format, latitude_longitude, download_url
            let lat_long = row[2]
                .replace("Latitude, Longitude: ", "")
                .replace(", ", "_");
            let download_url = &row[3];
            (
                format!("{}_{}.{}", timestamp_str, lat_long, ext),
                download_url,
            )
        };

        let path = Path::new(output_dir).join(filename);

        if path.exists() {
            debug!("  * File already exists; skipping download: {:?}", path);
            skip_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }

        let mut resp = match ureq::get(download_url).call() {
            Ok(r) => r,
            Err(e) => {
                log_error(
                    gui_console,
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
                    gui_console,
                    format!("  * Error creating file {:?}: {}", path, e),
                );
                error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        };

        match copy(&mut resp.body_mut().as_reader(), &mut file) {
            Ok(_) => {
                debug!("  * Downloaded {}", download_url);
                success_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                log_error(
                    gui_console,
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
                    error!("Error sending status to GUI: {}", e);
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
                error!("Error sending status to GUI: {}", e);
            });
        }
        None => {}
    }

    log_message(
        gui_console,
        format!("Finished processing {} links", records.len()),
    );
    if success_count > 0 {
        log_message(gui_console, format!("  - Success: {} files", records.len()));
    }
    if error_count > 0 {
        log_error(gui_console, format!("  - Error: {} files", error_count));
    }
    if skip_count > 0 {
        log_message(
            gui_console,
            format!("  - Skipped: {} files (already existed)", skip_count),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_look_for_item_found() {
        let buffer = b"hello world table tag here";
        let item = b"table";

        match look_for_item(buffer, item, false) {
            SearchResult::Found(index) => assert_eq!(index, 12),
            _ => panic!("Expected to find item at index 12"),
        }
    }

    #[test]
    fn test_look_for_item_not_found() {
        let buffer = b"hello world";
        let item = b"missing";

        match look_for_item(buffer, item, true) {
            SearchResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_look_for_item_not_found_with_unprocessed() {
        let buffer = b"hello world";
        let item = b"table";

        match look_for_item(buffer, item, false) {
            SearchResult::NotFoundWithUnprocessed(unprocessed) => {
                assert_eq!(unprocessed, 4); // item.len() - 1
            }
            _ => panic!("Expected NotFoundWithUnprocessed"),
        }
    }

    // #[test]
    // fn test_look_for_item_buffer_smaller_than_item() {
    //     let buffer = b"hi";
    //     let item = b"table";

    //     match look_for_item(buffer, item, false) {
    //         SearchResult::NotFoundWithUnprocessed(unprocessed) => {
    //             assert_eq!(unprocessed, 2); // buffer.len()
    //         }
    //         _ => panic!("Expected NotFoundWithUnprocessed with buffer length"),
    //     }
    // }

    // #[test]
    // fn test_look_for_item_empty_inputs() {
    //     assert!(matches!(
    //         look_for_item(b"", b"item", false),
    //         SearchResult::NotFound
    //     ));
    //     assert!(matches!(
    //         look_for_item(b"buffer", b"", false),
    //         SearchResult::NotFound
    //     ));
    // }

    #[test]
    fn test_look_for_item_exact_match() {
        let buffer = b"table";
        let item = b"table";

        match look_for_item(buffer, item, false) {
            SearchResult::Found(index) => assert_eq!(index, 0),
            _ => panic!("Expected to find item at index 0"),
        }
    }

    #[test]
    fn test_look_for_item_at_end() {
        let buffer = b"hello table";
        let item = b"table";

        match look_for_item(buffer, item, false) {
            SearchResult::Found(index) => assert_eq!(index, 6),
            _ => panic!("Expected to find item at index 6"),
        }
    }

    #[test]
    fn test_look_for_item_partial_at_end_not_last() {
        let buffer = b"hello tab";
        let item = b"table";

        match look_for_item(buffer, item, false) {
            SearchResult::NotFoundWithUnprocessed(unprocessed) => {
                assert_eq!(unprocessed, 4); // item.len() - 1
            }
            _ => panic!("Expected NotFoundWithUnprocessed"),
        }
    }

    #[test]
    fn test_look_for_item_partial_at_end_is_last() {
        let buffer = b"hello tab";
        let item = b"table";

        match look_for_item(buffer, item, true) {
            SearchResult::NotFound => {}
            _ => panic!("Expected NotFound when is_last=true"),
        }
    }

    #[test]
    fn test_look_html() {
        let buffer = b"aslkdjflkasjdflk\n\n\nasdfasdf<><table>sadfasdf<tbody>";
        let item1 = b"<table>";
        let item2 = b"<tbody>";
        let mut curr_index = 0;
        match look_for_item(buffer, item1, false) {
            SearchResult::Found(index) => {
                assert_eq!(index, 29);
                curr_index += index + item1.len();
            }
            _ => panic!("Expected to find item1 at index 29"),
        }
        match look_for_item(&buffer[curr_index..], item2, false) {
            SearchResult::Found(index) => assert_eq!(index, 8),
            _ => panic!("Expected to find item2 at index 8"),
        }
    }

    #[test]
    fn test_parse_html_snippet() {
        let test_file_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test")
            .join("test.html");

        println!("Test file path: {:?}", test_file_path);
        // Parse the headers and rows from this HTML snippet, starting at
        // the first <table> tag.
        match parse_memories_history_html(test_file_path.to_str().unwrap(), None) {
            Ok(records) => {
                // Assert the header record
                assert_eq!(records[0].len(), 4, "Expected 4 fields in header row");
                assert_eq!(
                    records[0].get(0).unwrap(),
                    "<b>Date</b>",
                    "Expected header row field 0 to be (right)"
                );
                assert_eq!(
                    records[0].get(1).unwrap(),
                    "<b>Media Type</b>",
                    "Expected header row field 1 to be (right)"
                );
                assert_eq!(
                    records[0].get(2).unwrap(),
                    "<b>Location</b>",
                    "Expected header row field 2 to be (right)"
                );
                assert_eq!(
                    records[0].get(3).unwrap(),
                    "<b></b>",
                    "Expected header row field 3 to be (right)"
                );

                println!("Header Row 0: {:?}", records[0]);
                println!("Row 0: {:?}", records[1]);

                // Assert the first record of data\
                assert_eq!(records[1].len(), 4, "Expected 4 fields in record 0");
                assert_eq!(
                    records[1].get(0).unwrap(),
                    "2026-01-13 01:55:38 UTC",
                    "Expected record 0 field 0 to be (right)"
                );
                assert_eq!(
                    records[1].get(1).unwrap(),
                    "Image",
                    "Expected record 0 field 1 to be (right)"
                );
                assert_eq!(
                    records[1].get(2).unwrap(),
                    "Latitude, Longitude: 40.25548, -111.645325",
                    "Expected record 0 field 2 to be (right)"
                );
                assert_eq!(
                    records[1].get(3).unwrap(),
                    "https://us-east1-aws.api.snapchat.com/dmd/mm?uid=bogus-1&sid=bogus-2&mid=bogus-3&ts=1768335041137&sig=bogus-4",
                    "Expected record 0 field 3 to be (right)"
                );

                // For this test, we expect 0 records since the HTML is incomplete
                assert_eq!(records.len(), 4, "Expected (right) total records");
            }
            Err(e) => {
                panic!("Error loading or parsing HTML snippet: {}", e);
            }
        }
    }
}
