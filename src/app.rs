//! src/app.rs
//!
//! Live LoRa telemetry visualization app
//! Reads data directly from the serial port (e.g., /dev/ttyACM0)
//! Parses actual telemetry lines received from the CanSat receiver,
//! and updates real-time graphs (Message #, RSSI, Payload).
//!
//! # Top-Level Application (`app.rs`)
//!
//! Constructs graphs, starts the remote control thread, and runs the UI main
//! loop for the terminal-based graphing application.
//!
//! ## Overview
//! The application:
//! - Renders multiple live-updating graphs in a terminal UI.
//! - Provides keyboard controls per graph.
//! - Spawns a TCP control server that accepts line-based ASCII commands.
//!
//! This document explains how to run the application, keyboard controls, the
//! TCP control protocol, autoscale/smoothing behavior, customization options,
//! and debugging tips.
//!
//! # Building and Running
//!
//! 1. From the project root:
//!    ```text
//!    cargo build --release
//!    ```
//!
//! 2. Run the app directly:
//!    ```text
//!    cargo run --release
//!    ```
//!
//! ### Environment Notes
//! - Terminal UI uses the `ratatui` and `crossterm` crates.
//! - Remote control server binds to `127.0.0.1:4000` by default.  
//!   Change this by editing the string passed to `remote_server(...)` inside
//!   the thread spawn.
//!
//! # Keyboard Controls (Interactive)
//!
//! - **Tab** — Cycle focus among graphs. The focused graph’s Info panel is highlighted.
//! - **a** — Toggle autoscale for the focused graph. Autoscale clears any locked bounds.
//! - **s** — Cycle smoothing presets for the focused graph.  
//!   Presets: `0.0, 0.25, 0.5, 0.75, 1.0` (0.0 = slow, 1.0 = instant).
//! - **l** — Lock/unlock the current graph’s Y-axis bounds.
//! - **q** — Quit and restore terminal state.
//!
//! # Remote TCP Protocol (ASCII, Line-Based)
//!
//! A small TCP server runs in a dedicated thread.  
//! Each received line is parsed as a whitespace-separated ASCII command.  
//! The server replies with one line per command (`OK` or `ERR <msg>`).
//!
//! **Default bind address:** `127.0.0.1:4000`
//!
//! ## Supported Commands
//!
//! - `toggle autoscale <idx>`  
//!   Toggle autoscale for graph `<idx>`.
//!
//! - `set smoothing <idx> <val>`  
//!   Set smoothing for graph `<idx>` to `<val>` (clamped to `[0.0, 1.0]`).
//!
//! - `lock <idx>`  
//!   Lock the current view bounds.  
//!   Returns `ERR no_bounds` if the bounds are not yet initialized.
//!
//! - `unlock <idx>`  
//!   Clear locked bounds and resume autoscale if enabled.
//!
//! - `quit`  
//!   Replies `OK bye` and closes the connection.
//!
//! ## Example Sessions
//!
//! Toggle autoscale on graph 1:
//! ```text
//! $ nc 127.0.0.1 4000
//! toggle autoscale 1
//! OK
//! ```
//!
//! Set smoothing to 0.5 on graph 2:
//! ```text
//! $ nc 127.0.0.1 4000
//! set smoothing 2 0.5
//! OK
//! ```
//!
//! Lock bounds on graph 0:
//! ```text
//! $ nc 127.0.0.1 4000
//! lock 0
//! OK
//! ```
//!
//! Unlock graph 0:
//! ```text
//! $ nc 127.0.0.1 4000
//! unlock 0
//! OK
//! ```
//!
//! ### Notes
//! - `<idx>` is the index in the `Vec<SharedGraph>` created in `run()`.
//! - Multiple clients can connect concurrently; each connection gets a dedicated thread.
//! - Errors return helpful `ERR` messages.
//!
//! # Internals: Autoscale, Smoothing, Hysteresis, Locking
//!
//! ### Target Bounds
//! The renderer computes target `(ymin, ymax)` from visible data, applying:
//! - 10% padding for non-flat ranges,
//! - magnitude-based padding for flat data.
//!
//! ### Interpolation (Smoothing)
//! `interp_bounds(current, target, alpha)` moves bounds toward the target.
//! - `1.0` = instant snap  
//! - smaller values = smoother transitions
//!
//! ### Hysteresis
//! - When data moves outside current bounds → **expansion** happens quickly.
//! - When data stays inside bounds → a `stable_count` must be reached before
//!   shrinking occurs, preventing jitter.
//!
//! ### Locked Bounds
//! - When locked, the stored bounds override autoscale completely.
//! - Unlocking restores autoscale (if enabled).
//!
//! # Extending the Application
//!
//! - **Adding graphs:**  
//!   Modify the graph configuration values (`cfg1`, `cfg2`, etc.) before creating
//!   `SharedGraph` instances. Index order determines the remote `<idx>` values.
//!
//! # Example Workflow
//!
//! 1. `cargo run`  
//! 2. Press `Tab` until the *Spikes* graph is focused  
//! 3. Press `s` several times and observe smoothing behavior  
//! 4. From another terminal:  
//!    ```text
//!    $ nc 127.0.0.1 4000
//!    lock 1
//!    OK
//!    ```
//! 5. Press `a` to toggle autoscale  
//! 6. Press `q` to exit  
//!
//! # Implementation Note
//!
//! `run()` constructs shared graph objects and spawns the control server so the
//! UI panels remain focused solely on rendering.  
//! This clean separation (UI vs. data vs. remote control) keeps the system
//! maintainable and easy to extend.

use std::error::Error;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::graph::GraphConfig;
use crate::graph::shared::{GraphShared, SharedGraph};
use crate::net::remote::remote_server;
use crate::panels::{GraphPanel, HistoryPanel, InfoPanel, ParagraphPanel, TitlePanel};
use crate::ui::{Node, group, leaf};

use ratatui::style::Color;

/// Spawn a thread that reads telemetry from a serial port (e.g., /dev/ttyACM0),
/// parses each line for message number, RSSI, and RSSI_PACKET,
/// and pushes them into the corresponding shared graphs.
fn start_serial_reader(
    port_name: &str,
    g_msg: SharedGraph,
    g_rssi: SharedGraph,
    g_rssi_packet: SharedGraph,
) {
    let port_name = port_name.to_string();
    thread::spawn(move || {
        let baud_rate = 115_200;
        println!("Opening serial port {} @ {} baud", port_name, baud_rate);
        let port = match serialport::new(&port_name, baud_rate)
            .timeout(Duration::from_secs(10))
            .open()
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to open serial port {}: {:?}", port_name, e);
                return;
            }
        };
        let reader = BufReader::new(port);
        println!("Serial reader started on {}", port_name);
        for line_res in reader.lines() {
            match line_res {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    // Parse the line for message number, RSSI, and RSSI_PACKET
                    let (maybe_msgnum, maybe_rssi, maybe_rssi_packet) =
                        parse_telemetry_line(trimmed);

                    // Update message graph
                    if let Some(msgnum) = maybe_msgnum {
                        if let Ok(mut gm) = g_msg.write() {
                            let x = gm.data.history.back().map(|(x, _)| x + 1.0).unwrap_or(0.0);
                            gm.data.push_point(x, msgnum as f64);
                        }
                    }
                    // Update RSSI graph
                    if let Some(rssi) = maybe_rssi {
                        if let Ok(mut gr) = g_rssi.write() {
                            let x = gr.data.history.back().map(|(x, _)| x + 1.0).unwrap_or(0.0);
                            gr.data.push_point(x, rssi);
                        }
                    }
                    // Update RSSI_PACKET graph
                    if let Some(rssi_packet) = maybe_rssi_packet {
                        if let Ok(mut gr) = g_rssi_packet.write() {
                            let x = gr.data.history.back().map(|(x, _)| x + 1.0).unwrap_or(0.0);
                            gr.data.push_point(x, rssi_packet);
                        }
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) => {
                    eprintln!("Error reading serial data: {:?}", e);
                    break;
                }
            }
        }
        println!("Serial reader exiting");
    });
}

/// Parse a telemetry line and extract message number, RSSI, and RSSI_PACKET.
///
/// Example accepted format:
/// ----------------------------------------
/// Received: MSG 9 RSSI -94.5
/// RSSI_PACKET: -93.5 dBm
/// ACK sent back automatically.
/// ----------------------------------------
///
/// Returns (Option<msgnum>, Option<rssi>, Option<rssi_packet>)
fn parse_telemetry_line(line: &str) -> (Option<u64>, Option<f64>, Option<f64>) {
    let mut msgnum = None;
    let mut rssi = None;
    let mut rssi_packet = None;

    // Split the line into parts
    let lines: Vec<&str> = line.lines().collect();

    // Iterate through each line
    for l in lines {
        let trimmed = l.trim();

        // Parse "Received: MSG 9 RSSI -94.5"
        if trimmed.starts_with("Received: MSG") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                // Extract message number (e.g., "9")
                if let Ok(num) = parts[2].parse::<u64>() {
                    msgnum = Some(num);
                }
                // Extract RSSI (e.g., "-94.5")
                if let Ok(val) = parts[4].parse::<f64>() {
                    rssi = Some(val);
                }
            }
        }
        // Parse "RSSI_PACKET: -93.5 dBm"
        else if trimmed.starts_with("RSSI_PACKET:") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                // Extract RSSI_PACKET (e.g., "-93.5")
                if let Ok(val) = parts[1].parse::<f64>() {
                    rssi_packet = Some(val);
                }
            }
        }
    }

    (msgnum, rssi, rssi_packet)
}

pub fn run() -> Result<(), Box<dyn Error>> {
    // Graph configuration
    let cfg_msg = GraphConfig::new(10, 1_000, (0.0, 1000.0));
    let cfg_rssi = GraphConfig::new(10, 1_000, (-120.0, 0.0));
    let cfg_rssi_packet = GraphConfig::new(10, 1_000, (-120.0, 0.0));

    // Shared graphs
    let g_msg: SharedGraph = Arc::new(RwLock::new(GraphShared::new(
        cfg_msg,
        "Msg #",
        Color::Magenta,
        false,
        0.35,
    )));
    let g_rssi: SharedGraph = Arc::new(RwLock::new(GraphShared::new(
        cfg_rssi,
        "RSSI ACK (dBm)",
        Color::Cyan,
        false,
        1.0,
    )));
    let g_rssi_packet: SharedGraph = Arc::new(RwLock::new(GraphShared::new(
        cfg_rssi_packet,
        "RSSI PACKET (dBm)",
        Color::Yellow,
        false,
        1.0,
    )));

    let graphs: Vec<SharedGraph> = vec![g_msg.clone(), g_rssi.clone(), g_rssi_packet.clone()];

    // Remote control thread
    {
        let graphs_for_thread = graphs.clone();
        thread::spawn(move || remote_server("127.0.0.1:4000", graphs_for_thread));
    }

    // Start serial reader
    start_serial_reader(
        "/dev/ttyACM0",
        g_msg.clone(),
        g_rssi.clone(),
        g_rssi_packet.clone(),
    );

    // UI setup
    let mut terminal = ratatui::init();
    let mut focused = 0usize;
    let smoothing_presets = [0.0, 0.25, 0.5, 0.75, 1.0];
    let frame_time = Duration::from_millis(100);
    let mut running = true;

    while running {
        let frame_start = std::time::Instant::now();

        // This basically set how the graphs should be `packaged` bc it mainly put other panels next to
        // the graphs
        // Don't bother with this. It's details.
        let mut left_children: Vec<Node> = Vec::new();
        for i in 0..graphs.len() {
            let gp =
                leaf(Box::new(GraphPanel::new(graphs[i].clone())) as Box<dyn crate::ui::Panel>);
            let hist =
                leaf(Box::new(HistoryPanel::new(graphs[i].clone())) as Box<dyn crate::ui::Panel>);
            let mut info_panel = InfoPanel::new(graphs[i].clone());
            info_panel.highlighted = i == focused;
            let info = leaf(Box::new(info_panel) as Box<dyn crate::ui::Panel>);

            let region = group(
                ratatui::layout::Direction::Vertical,
                vec![
                    ratatui::layout::Constraint::Percentage(70),
                    ratatui::layout::Constraint::Percentage(30),
                ],
                vec![
                    gp,
                    group(
                        ratatui::layout::Direction::Horizontal,
                        vec![
                            ratatui::layout::Constraint::Percentage(60),
                            ratatui::layout::Constraint::Percentage(40),
                        ],
                        vec![hist, info],
                    ),
                ],
            );
            left_children.push(region);
        }

        // That's the part of code that set up the `Controls` panel
        let extra = leaf(Box::new(ParagraphPanel::new(
            "TAB=Focus  A=Autoscale  S=Smoothing  L=Lock bounds  Q=Quit",
            "Controls",
        )) as Box<dyn crate::ui::Panel>);

        // This set up the main interface layout
        let root = group(
            // These constraints are applied vertically to the whole terminal window
            ratatui::layout::Direction::Vertical,
            vec![
                // 3 lines
                ratatui::layout::Constraint::Length(3),
                // 80% of the screen
                ratatui::layout::Constraint::Percentage(80),
                // minimum 3 lines but could be more
                ratatui::layout::Constraint::Min(3),
            ],
            vec![
                // Leaf are basically single panels
                // This one set up the title
                // And it take the place of our first vertical constraint (the 3 lines)
                leaf(
                    Box::new(TitlePanel::new("Live CanSat Telemetry")) as Box<dyn crate::ui::Panel>
                ),
                // This one is kinda self-explanatory
                group(
                    // Divide the second vertical constraint in a horizontal way
                    ratatui::layout::Direction::Horizontal,
                    // The right part take 60% and the left 40%
                    vec![
                        ratatui::layout::Constraint::Percentage(60),
                        ratatui::layout::Constraint::Percentage(40),
                    ],
                    // Now what to put into these panels ?
                    // Here where puting the actual panels where we layed out everything
                    vec![
                        // This is the the right part
                        // It group every graph and give it a space
                        group(
                            ratatui::layout::Direction::Vertical,
                            vec![
                                ratatui::layout::Constraint::Percentage(33),
                                ratatui::layout::Constraint::Percentage(33),
                                ratatui::layout::Constraint::Percentage(34),
                            ],
                            // Here we call the thing that contain the grpahs
                            left_children,
                        ),
                        // Here we call the extra, which is the `Control` panel that we set up earlier
                        extra,
                    ],
                ),
                // You should have understood how to set up everything by now
                leaf(Box::new(ParagraphPanel::new("Press Q to quit.", "Footer"))
                    as Box<dyn crate::ui::Panel>),
            ],
        );

        terminal.draw(|f| root.draw(f, f.area()))?;

        // Keyboard controls
        while crossterm::event::poll(Duration::from_millis(0))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Char('q') => running = false,
                    crossterm::event::KeyCode::Tab => focused = (focused + 1) % graphs.len(),
                    crossterm::event::KeyCode::Char('a') => {
                        let mut g = graphs[focused].write().unwrap();
                        g.autoscale = !g.autoscale;
                        if g.autoscale {
                            g.locked_bounds = None;
                        }
                    }
                    crossterm::event::KeyCode::Char('s') => {
                        let mut g = graphs[focused].write().unwrap();
                        let idx = smoothing_presets
                            .iter()
                            .position(|&v| (v - g.smoothing).abs() < 1e-9)
                            .unwrap_or(0);
                        g.smoothing = smoothing_presets[(idx + 1) % smoothing_presets.len()];
                    }
                    crossterm::event::KeyCode::Char('l') => {
                        let mut g = graphs[focused].write().unwrap();
                        if g.locked_bounds.is_some() {
                            g.locked_bounds = None;
                        } else {
                            g.locked_bounds = g.view.current_bounds;
                        }
                    }
                    _ => {}
                }
            }
        }

        if !running {
            break;
        }

        let elapsed = frame_start.elapsed();
        if elapsed < frame_time {
            thread::sleep(frame_time - elapsed);
        }
    }

    ratatui::restore();
    Ok(())
}
