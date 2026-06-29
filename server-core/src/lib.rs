use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use log::trace;

mod client;
mod server;

use crate::server::Server;

pub use messages::packets::EventValue;

const INIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum CoreEvent {
    EmitEvent {
        player_id: i32,
        event: String,
        arguments: String,
    },
    PlayerInitialized {
        player_id: i32,
        success: bool,
    },
    BrowserCreated {
        player_id: i32,
        browser_id: u32,
        code: i32,
    },
}

#[derive(Debug)]
pub(crate) enum ServerEvent {
    EmitEvent {
        player_id: i32,
        event: String,
        arguments: String,
    },
    PlayerConnected(i32),
    BrowserCreated {
        player_id: i32,
        browser_id: u32,
        code: i32,
    },
}

pub struct ServerCore {
    server: Arc<Mutex<Server>>,
    event_rx: Receiver<ServerEvent>,
    await_connect: std::collections::HashMap<i32, Instant>,
    ips: std::collections::HashMap<i32, IpAddr>,
    pending_events: VecDeque<CoreEvent>,
}

impl ServerCore {
    pub fn new(addr: SocketAddr) -> Result<Self, String> {
        let server = Server::new(addr)?;

        let event_rx = {
            let s = server.lock().unwrap();
            s.receiver()
        };

        Ok(Self {
            server,
            event_rx,
            await_connect: std::collections::HashMap::new(),
            ips: std::collections::HashMap::new(),
            pending_events: VecDeque::new(),
        })
    }

    pub fn allow_connection(&mut self, player_id: i32, addr: IpAddr) {
        trace!("allow_connection {} {:?}", player_id, addr);

        self.ips.insert(player_id, addr);

        let already_connected = {
            let server = self.server.lock().unwrap();
            server.has_plugin(player_id)
        };

        {
            let mut server = self.server.lock().unwrap();
            server.allow_connection(player_id, addr);
        }

        if !already_connected {
            self.await_connect.insert(player_id, Instant::now());
        }
    }

    pub fn remove_connection(&mut self, player_id: i32) {
        trace!("remove_connection {}", player_id);

        let ip = self.ips.remove(&player_id);

        {
            let mut server = self.server.lock().unwrap();
            server.remove_connection(player_id, ip);
        }

        self.await_connect.remove(&player_id);
    }

    pub fn create_browser(
        &mut self, player_id: i32, browser_id: i32, url: String, hidden: bool, focused: bool,
    ) {
        let mut server = self.server.lock().unwrap();
        server.create_browser(player_id, browser_id, url, hidden, focused);
    }

    pub fn destroy_browser(&mut self, player_id: i32, browser_id: i32) {
        let mut server = self.server.lock().unwrap();
        server.destroy_browser(player_id, browser_id);
    }

    pub fn hide_browser(&self, player_id: i32, browser_id: i32, hide: bool) {
        let server = self.server.lock().unwrap();
        server.hide_browser(player_id, browser_id, hide);
    }

    pub fn focus_browser(&self, player_id: i32, browser_id: i32, focused: bool) {
        let server = self.server.lock().unwrap();
        server.focus_browser(player_id, browser_id, focused);
    }

    pub fn emit_event(&self, player_id: i32, event: &str, arguments: Vec<EventValue<'static>>) {
        let server = self.server.lock().unwrap();
        server.emit_event(player_id, event, arguments);
    }

    pub fn always_listen_keys(&self, player_id: i32, browser_id: i32, listen: bool) {
        let server = self.server.lock().unwrap();
        server.always_listen_keys(player_id, browser_id, listen);
    }

    pub fn has_plugin(&self, player_id: i32) -> bool {
        let server = self.server.lock().unwrap();
        server.has_plugin(player_id)
    }

    pub fn create_external_browser(
        &self, player_id: i32, browser_id: i32, texture: String, url: String, scale: i32,
    ) {
        let server = self.server.lock().unwrap();
        server.create_external_browser(player_id, browser_id, texture, url, scale);
    }

    pub fn append_to_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        let server = self.server.lock().unwrap();
        server.append_to_object(player_id, browser_id, object_id);
    }

    pub fn remove_from_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        let server = self.server.lock().unwrap();
        server.remove_from_object(player_id, browser_id, object_id);
    }

    pub fn toggle_dev_tools(&self, player_id: i32, browser_id: i32, enabled: bool) {
        let server = self.server.lock().unwrap();
        server.toggle_dev_tools(player_id, browser_id, enabled);
    }

    pub fn set_audio_settings(
        &self, player_id: i32, browser_id: u32, max_distance: f32, reference_distance: f32,
    ) {
        let server = self.server.lock().unwrap();
        server.set_audio_settings(player_id, browser_id, max_distance, reference_distance);
    }

    pub fn load_url(&self, player_id: i32, browser_id: u32, url: String) {
        let server = self.server.lock().unwrap();
        server.load_url(player_id, browser_id, url);
    }

    pub fn poll_event(&mut self) -> Option<CoreEvent> {
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }

        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                ServerEvent::EmitEvent {
                    player_id,
                    event,
                    arguments,
                } => {
                    self.pending_events.push_back(CoreEvent::EmitEvent {
                        player_id,
                        event,
                        arguments,
                    });
                }

                ServerEvent::PlayerConnected(player_id) => {
                    trace!("poll_event::PlayerConnected({})", player_id);

                    if self.await_connect.remove(&player_id).is_some() {
                        self.pending_events.push_back(CoreEvent::PlayerInitialized {
                            player_id,
                            success: true,
                        });
                    }
                }

                ServerEvent::BrowserCreated {
                    player_id,
                    browser_id,
                    code,
                } => {
                    self.pending_events.push_back(CoreEvent::BrowserCreated {
                        player_id,
                        browser_id,
                        code,
                    });
                }
            }
        }

        self.notify_timeouts();
        self.pending_events.pop_front()
    }

    fn notify_timeouts(&mut self) {
        let mut keys = Vec::new();

        for (&player_id, timing) in self.await_connect.iter() {
            if timing.elapsed() >= INIT_TIMEOUT {
                keys.push(player_id);
            }
        }

        for player_id in keys {
            self.await_connect.remove(&player_id);
            self.pending_events.push_back(CoreEvent::PlayerInitialized {
                player_id,
                success: false,
            });
        }
    }
}
