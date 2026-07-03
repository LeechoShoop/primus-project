use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs},
    Terminal,
};
use std::{error::Error, io};
use std::time::Duration;
use tokio::sync::mpsc;

use primus_client::rpc::NodeClient;
use primus_sdk::{Wallet, TransactionBuilder, PROTOCOL_MIN_FEE};

/// Read the seed node address from .primus_config.toml if it exists.
/// Returns (host, port).
fn load_node_addr_from_config() -> Option<(String, u16)> {
    let text = std::fs::read_to_string(".primus_config.toml").ok()?;
    let mut host = String::new();
    let mut port: u16 = 9000;
    for line in text.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("public_ip = ") {
            let s = val.trim().trim_matches('"');
            if !s.is_empty() { host = s.to_string(); }
        } else if let Some(val) = line.strip_prefix("port = ") {
            port = val.trim().parse().unwrap_or(9000);
        }
    }
    if !host.is_empty() { Some((host, port)) } else { None }
}

enum AppTab {
    Dashboard,
    Send,
    Logs,
}

enum AppEvent {
    #[allow(dead_code)]
    Tick,
    NodeStatus(String),
    BalanceUpdate(u64),
    LogMsg(String),
    WalletLoaded(String),
}

enum UiCommand {
    SendTx { to: String, amount: u64 },
    GenerateWallet,
}

struct App {
    pub tab: AppTab,
    pub address_input: String,
    pub amount_input: String,
    pub is_typing_address: bool,
    pub logs: Vec<String>,
    pub balance: Option<u64>,
    pub status: String,
    pub wallet_address: String,
    #[allow(dead_code)]
    pub node_addr: String,
}

impl App {
    fn new(node_addr: &str) -> App {
        App {
            tab: AppTab::Dashboard,
            address_input: String::new(),
            amount_input: String::new(),
            is_typing_address: true,
            logs: vec!["Client started.".to_string()],
            balance: None,
            status: "Connecting...".to_string(),
            wallet_address: "Not loaded".to_string(),
            node_addr: node_addr.to_string(),
        }
    }

    fn next_tab(&mut self) {
        self.tab = match self.tab {
            AppTab::Dashboard => AppTab::Send,
            AppTab::Send => AppTab::Logs,
            AppTab::Logs => AppTab::Dashboard,
        };
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Parse optional --node IP:PORT argument
    let args: Vec<String> = std::env::args().collect();
    let node_addr: (String, u16) = {
        let mut addr = None;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--node" && i + 1 < args.len() {
                let s = &args[i + 1];
                if let Ok(sock) = s.parse::<std::net::SocketAddr>() {
                    addr = Some((sock.ip().to_string(), sock.port()));
                } else if let Some((h, p)) = s.split_once(':') {
                    addr = Some((h.to_string(), p.parse().unwrap_or(9000)));
                }
                break;
            }
            i += 1;
        }
        addr
            .or_else(load_node_addr_from_config)
            .unwrap_or_else(|| ("127.0.0.1".to_string(), 9000))
    };
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = App::new(&format!("{}:{}", node_addr.0, node_addr.1));

    // Setup channels
    let (tx, mut rx) = mpsc::channel(100);
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiCommand>(10);
    
    // Background poller
    let tx_clone = tx.clone();
    let node_host = node_addr.0.clone();
    let node_port = node_addr.1;
    tokio::spawn(async move {
        // Try to load default wallet
        let mut wallet = Wallet::load(std::path::Path::new("my.wallet")).ok();
        let mut wallet_addr = if let Some(ref w) = wallet {
            let _ = tx_clone.send(AppEvent::WalletLoaded(w.address.clone())).await;
            Some(w.address.clone())
        } else {
            let _ = tx_clone.send(AppEvent::LogMsg("my.wallet not found. Press 'g' to generate a new wallet.".into())).await;
            None
        };

        let mut interval = tokio::time::interval(Duration::from_secs(2));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match NodeClient::new_auto(&node_host, node_port).await {
                        Ok(mut client) => {
                            let _ = tx_clone.send(AppEvent::NodeStatus(format!("Connected ({}:{})", node_host, node_port))).await;
                            
                            if let Some(ref addr) = wallet_addr {
                                match client.get_atom_state(addr).await {
                                    Ok(state) => {
                                        let _ = tx_clone.send(AppEvent::BalanceUpdate(state.mass)).await;
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(AppEvent::LogMsg(format!("Error fetching state: {}", e))).await;
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            let _ = tx_clone.send(AppEvent::NodeStatus("Disconnected".into())).await;
                        }
                    }
                }
                Some(cmd) = ui_rx.recv() => {
                    match cmd {
                        UiCommand::SendTx { to, amount } => {
                            if let Some(ref w) = wallet {
                                let _ = tx_clone.send(AppEvent::LogMsg(format!("Sending {} mass to {}...", amount, to))).await;
                                match NodeClient::new_auto(&node_host, node_port).await {
                                    Ok(mut client) => {
                                        match client.get_atom_state(&w.address).await {
                                            Ok(state) => {
                                                if state.mass < amount + PROTOCOL_MIN_FEE {
                                                    let _ = tx_clone.send(AppEvent::LogMsg("Insufficient balance!".into())).await;
                                                    continue;
                                                }
                                                
                                                if let Ok(recipient_pk) = Wallet::decode_address(&to) {
                                                    let builder = TransactionBuilder::new(w)
                                                        .recipient(recipient_pk)
                                                        .amount(amount)
                                                        .fee(PROTOCOL_MIN_FEE)
                                                        .sender_mass(state.mass)
                                                        .sender_last_hash(state.last_hash)
                                                        .sender_nonce(state.nonce)
                                                        .sender_element(state.element);
                                                    
                                                    match client.broadcast_tx(w, builder).await {
                                                        Ok(msg) => { let _ = tx_clone.send(AppEvent::LogMsg(format!("Tx Broadcasted: {}", msg))).await; }
                                                        Err(e) => { let _ = tx_clone.send(AppEvent::LogMsg(format!("Broadcast Error: {}", e))).await; }
                                                    }
                                                } else {
                                                    let _ = tx_clone.send(AppEvent::LogMsg("Invalid recipient address format!".into())).await;
                                                }
                                            }
                                            Err(e) => { let _ = tx_clone.send(AppEvent::LogMsg(format!("Failed to get sender state: {}", e))).await; }
                                        }
                                    }
                                    Err(_) => { let _ = tx_clone.send(AppEvent::LogMsg("Cannot send: Node disconnected.".into())).await; }
                                }
                            } else {
                                let _ = tx_clone.send(AppEvent::LogMsg("Cannot send: No wallet loaded!".into())).await;
                            }
                        }
                        UiCommand::GenerateWallet => {
                            if wallet.is_none() {
                                let _ = tx_clone.send(AppEvent::LogMsg("Generating wallet...".into())).await;
                                match Wallet::generate(12, 0) {
                                    Ok(w) => {
                                        if let Err(e) = w.save(std::path::Path::new("my.wallet")) {
                                            let _ = tx_clone.send(AppEvent::LogMsg(format!("Failed to save wallet: {}", e))).await;
                                        } else {
                                            let mnemonic = w.get_mnemonic().to_string();
                                            wallet_addr = Some(w.address.clone());
                                            let _ = tx_clone.send(AppEvent::WalletLoaded(w.address.clone())).await;
                                            let _ = tx_clone.send(AppEvent::LogMsg(format!("✅ Wallet created! Mnemonic: {}", mnemonic))).await;
                                            wallet = Some(w);
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(AppEvent::LogMsg(format!("Failed to generate wallet: {}", e))).await;
                                    }
                                }
                            } else {
                                let _ = tx_clone.send(AppEvent::LogMsg("⚠️ Wallet already exists.".into())).await;
                            }
                        }
                    }
                }
            }
        }
    });

    let res = run_app(&mut terminal, app, &mut rx, ui_tx).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App, rx: &mut mpsc::Receiver<AppEvent>, ui_tx: mpsc::Sender<UiCommand>) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &app))?;

        // Try to process background events without blocking
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::NodeStatus(s) => app.status = s,
                AppEvent::BalanceUpdate(b) => app.balance = Some(b),
                AppEvent::WalletLoaded(a) => app.wallet_address = a,
                AppEvent::LogMsg(msg) => {
                    app.logs.push(msg);
                    if app.logs.len() > 50 { app.logs.remove(0); }
                }
                _ => {}
            }
        }

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Tab => app.next_tab(),
                    KeyCode::Char(c) => {
                        if let AppTab::Send = app.tab {
                            if app.is_typing_address {
                                app.address_input.push(c);
                            } else {
                                app.amount_input.push(c);
                            }
                        } else if c == 'g' {
                            let _ = ui_tx.try_send(UiCommand::GenerateWallet);
                        }
                    }
                    KeyCode::Backspace => {
                        if let AppTab::Send = app.tab {
                            if app.is_typing_address {
                                app.address_input.pop();
                            } else {
                                app.amount_input.pop();
                            }
                        }
                    }
                    KeyCode::Enter => {
                        if let AppTab::Send = app.tab {
                            if let Ok(amount) = app.amount_input.parse::<u64>() {
                                let _ = ui_tx.try_send(UiCommand::SendTx {
                                    to: app.address_input.clone(),
                                    amount,
                                });
                                app.amount_input.clear();
                                app.address_input.clear();
                                app.tab = AppTab::Logs;
                            } else {
                                app.logs.push("Invalid amount!".into());
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Up => {
                        if let AppTab::Send = app.tab {
                            app.is_typing_address = !app.is_typing_address;
                        }
                    }
                    _ => {}
                }
            }
    }
}

fn ui(f: &mut ratatui::Frame, app: &App) {
    let size = f.size();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(vec![
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
        ])
        .split(size);

    let titles: Vec<Line> = vec![" Dashboard ", " Send ", " Logs "].into_iter().map(Line::from).collect();
    let selected_tab = match app.tab {
        AppTab::Dashboard => 0,
        AppTab::Send => 1,
        AppTab::Logs => 2,
    };

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" Primus-Client TUI "))
        .select(selected_tab)
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    
    f.render_widget(tabs, chunks[0]);

    match app.tab {
        AppTab::Dashboard => {
            let bal_str = match app.balance {
                Some(b) => format!("{} mass", b),
                None => "N/A".to_string()
            };
            let info = format!("Network Status: {}\nWallet: {}\nBalance: {}", app.status, app.wallet_address, bal_str);
            let p = Paragraph::new(info).block(Block::default().borders(Borders::ALL).title(" Wallet Info "));
            f.render_widget(p, chunks[1]);
        }
        AppTab::Send => {
            let inner_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
                .split(chunks[1]);

            let addr_style = if app.is_typing_address { Style::default().fg(Color::Yellow) } else { Style::default() };
            let amt_style = if !app.is_typing_address { Style::default().fg(Color::Yellow) } else { Style::default() };

            let p_addr = Paragraph::new(app.address_input.as_str())
                .style(addr_style)
                .block(Block::default().borders(Borders::ALL).title(" Recipient Address "));
            f.render_widget(p_addr, inner_chunks[0]);

            let p_amt = Paragraph::new(app.amount_input.as_str())
                .style(amt_style)
                .block(Block::default().borders(Borders::ALL).title(" Amount "));
            f.render_widget(p_amt, inner_chunks[1]);
        }
        AppTab::Logs => {
            let log_text = app.logs.join("\n");
            let p = Paragraph::new(log_text).block(Block::default().borders(Borders::ALL).title(" Event Log "));
            f.render_widget(p, chunks[1]);
        }
    }

    let footer = Paragraph::new("Press 'q' to quit, 'Tab' to switch views")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
