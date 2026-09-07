//! App state machine for the TT terminal UI.

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::api::{Client, Repo, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Login,
    Home,
    Repos,
    Inbox,
    Channels,
    Compute,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginField {
    Handle,
    DisplayName,
    Passphrase,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    SignIn,
    Register,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedConfig {
    pub server: String,
    pub token: Option<String>,
    pub handle: Option<String>,
}

/// Flattened repo row for the Repos list.
#[derive(Debug, Clone)]
pub struct RepoRow {
    pub workspace_id: String,
    pub workspace_slug: String,
    pub workspace_name: String,
    pub repo: Repo,
}

pub struct App {
    pub screen: Screen,
    pub client: Client,
    pub handle: String,
    pub passphrase: String,
    pub server: String,
    pub login_field: LoginField,
    pub auth_mode: AuthMode,
    pub display_name: String,
    pub status: String,
    pub error: Option<String>,
    pub should_quit: bool,
    pub menu_idx: usize,
    pub list_idx: usize,
    pub body: String,
    pub logged_in: bool,
    pub workspaces: Vec<Workspace>,
    pub repos: Vec<RepoRow>,
    pub selected_workspace: usize,
    /// Last drawn terminal area (for mouse hit-testing).
    pub frame_area: Rect,
    pub modal: Modal,
}

/// Overlay forms for create/delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    None,
    CreateWorkspace {
        name: String,
        slug: String,
        field: WsField,
    },
    CreateRepo {
        name: String,
        description: String,
        is_private: bool,
        field: RepoField,
    },
    ConfirmDeleteRepo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsField {
    Name,
    Slug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoField {
    Name,
    Description,
    Visibility,
}

impl App {
    pub async fn load() -> Result<Self> {
        let cfg = load_config();
        let server = if cfg.server.is_empty() {
            "http://127.0.0.1:8080".into()
        } else {
            cfg.server.clone()
        };
        let client = Client::new(&server).with_token(cfg.token.clone());
        let logged_in = cfg.token.is_some();
        let mut app = Self {
            screen: if logged_in {
                Screen::Home
            } else {
                Screen::Login
            },
            client,
            handle: cfg.handle.unwrap_or_default(),
            passphrase: String::new(),
            server,
            login_field: LoginField::Handle,
            auth_mode: AuthMode::SignIn,
            display_name: String::new(),
            status: if logged_in {
                "session restored".into()
            } else {
                "sign in · press a to create an account".into()
            },
            error: None,
            should_quit: false,
            menu_idx: 0,
            list_idx: 0,
            body: String::new(),
            logged_in,
            workspaces: Vec::new(),
            repos: Vec::new(),
            selected_workspace: 0,
            frame_area: Rect::default(),
            modal: Modal::None,
        };
        if logged_in {
            let _ = app.refresh_home().await;
        }
        Ok(app)
    }

    pub fn input_mode(&self) -> bool {
        matches!(self.screen, Screen::Login) || self.modal != Modal::None
    }

    pub fn menu_items() -> &'static [(&'static str, Screen)] {
        &[
            ("Home", Screen::Home),
            ("Repos", Screen::Repos),
            ("Inbox", Screen::Inbox),
            ("Channels", Screen::Channels),
            ("Compute", Screen::Compute),
            ("Settings", Screen::Settings),
        ]
    }

    pub async fn handle_key(&mut self, key: KeyCode) -> Result<()> {
        self.error = None;
        if self.modal != Modal::None {
            return self.handle_modal_key(key).await;
        }
        match self.screen {
            Screen::Login => self.handle_login_key(key).await?,
            Screen::Repos => self.handle_repos_key(key).await?,
            _ => self.handle_nav_key(key).await?,
        }
        Ok(())
    }


    /// Process a terminal mouse/touch tap at (column, row).
    pub async fn handle_mouse(&mut self, column: u16, row: u16) -> Result<()> {
        let area = self.frame_area;
        if area.width == 0 || area.height == 0 {
            return Ok(());
        }

        // Login screen: tap bottom hint toggles mode; otherwise focus nearest field
        if self.screen == Screen::Login {
            // F2 equivalent — tap title bar
            if row <= area.y + 3 {
                self.toggle_auth_mode();
                return Ok(());
            }
            // Rough vertical field bands inside form
            let mid_y = area.y + area.height / 4;
            let rel = row.saturating_sub(mid_y);
            self.login_field = match self.auth_mode {
                AuthMode::SignIn => {
                    if rel < 3 {
                        LoginField::Handle
                    } else if rel < 6 {
                        LoginField::Passphrase
                    } else {
                        LoginField::Server
                    }
                }
                AuthMode::Register => {
                    if rel < 3 {
                        LoginField::Handle
                    } else if rel < 5 {
                        LoginField::DisplayName
                    } else if rel < 8 {
                        LoginField::Passphrase
                    } else {
                        LoginField::Server
                    }
                }
            };
            return Ok(());
        }

        // Shell: top tabs are rows [area.y, area.y+3)
        let tabs_bottom = area.y + 3;
        if row < tabs_bottom {
            let n = Self::menu_items().len() as u16;
            if n > 0 {
                let rel_x = column.saturating_sub(area.x);
                let slot = (rel_x * n) / area.width.max(1);
                let idx = (slot as usize).min(Self::menu_items().len() - 1);
                self.menu_idx = idx;
                self.screen = Self::menu_items()[idx].1;
                self.on_screen_enter().await?;
            }
            return Ok(());
        }

        if self.screen == Screen::Repos {
            // Same split as ui::draw_repos: 45% list | 55% detail under tabs
            let body_top = tabs_bottom;
            let body_height = area.height.saturating_sub(5); // tabs 3 + footer 2
            let list_w = (area.width as u32 * 45 / 100) as u16;
            let list_area = Rect {
                x: area.x,
                y: body_top,
                width: list_w,
                height: body_height,
            };
            if column >= list_area.x
                && column < list_area.x + list_area.width
                && row >= list_area.y
                && row < list_area.y + list_area.height
            {
                // border + title row → items start at y+1
                let inner_y = row.saturating_sub(list_area.y + 1);
                if !self.repos.is_empty() {
                    let idx = (inner_y as usize).min(self.repos.len() - 1);
                    self.list_idx = idx;
                    // show detail
                    if let Some(r) = self.repos.get(self.list_idx) {
                        self.status = format!("{}/{}", r.workspace_slug, r.repo.name);
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_modal_key(&mut self, key: KeyCode) -> Result<()> {
        let mut modal = std::mem::replace(&mut self.modal, Modal::None);
        match &mut modal {
            Modal::None => {}
            Modal::CreateWorkspace { name, slug, field } => match key {
                KeyCode::Esc => {
                    self.status = "cancelled".into();
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Down => {
                    *field = match field {
                        WsField::Name => WsField::Slug,
                        WsField::Slug => WsField::Name,
                    };
                    self.modal = modal;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    *field = match field {
                        WsField::Name => WsField::Slug,
                        WsField::Slug => WsField::Name,
                    };
                    self.modal = modal;
                }
                KeyCode::Enter => {
                    let name = name.trim().to_string();
                    let mut slug_s = slug.trim().to_string();
                    if name.is_empty() {
                        self.error = Some("workspace name required".into());
                        self.modal = modal;
                        return Ok(());
                    }
                    if slug_s.is_empty() {
                        slug_s = slugify(&name);
                    }
                    match self.client.create_workspace(&name, &slug_s, None).await {
                        Ok(ws) => {
                            self.status = format!("workspace {} created", ws.slug);
                            self.refresh_repos().await?;
                            if let Some(i) = self.workspaces.iter().position(|w| w.id == ws.id) {
                                self.selected_workspace = i;
                            }
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                            self.modal = modal;
                        }
                    }
                }
                KeyCode::Char(c) => {
                    match field {
                        WsField::Name => {
                            name.push(c);
                            let auto = slugify(name);
                            if slug.is_empty() || slugify(&name[..name.len().saturating_sub(1)]) == *slug
                            {
                                *slug = auto;
                            }
                        }
                        WsField::Slug => {
                            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                                slug.push(c.to_ascii_lowercase());
                            }
                        }
                    }
                    self.modal = modal;
                }
                KeyCode::Backspace => {
                    match field {
                        WsField::Name => {
                            name.pop();
                        }
                        WsField::Slug => {
                            slug.pop();
                        }
                    }
                    self.modal = modal;
                }
                _ => {
                    self.modal = modal;
                }
            },
            Modal::CreateRepo {
                name,
                description,
                is_private,
                field,
            } => match key {
                KeyCode::Esc => {
                    self.status = "cancelled".into();
                }
                KeyCode::Tab | KeyCode::Down => {
                    *field = match field {
                        RepoField::Name => RepoField::Description,
                        RepoField::Description => RepoField::Visibility,
                        RepoField::Visibility => RepoField::Name,
                    };
                    self.modal = modal;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    *field = match field {
                        RepoField::Name => RepoField::Visibility,
                        RepoField::Description => RepoField::Name,
                        RepoField::Visibility => RepoField::Description,
                    };
                    self.modal = modal;
                }
                KeyCode::Char(' ') if *field == RepoField::Visibility => {
                    *is_private = !*is_private;
                    self.modal = modal;
                }
                KeyCode::Enter => {
                    let name_s = name.trim().to_string();
                    if name_s.is_empty() {
                        self.error = Some("repo name required".into());
                        self.modal = modal;
                        return Ok(());
                    }
                    if self.workspaces.is_empty() {
                        self.error = Some("create a workspace first (w)".into());
                        return Ok(());
                    }
                    let desc = description.trim();
                    let desc_opt = if desc.is_empty() { None } else { Some(desc.to_string()) };
                    let priv_ = *is_private;
                    let idx = self.selected_workspace.min(self.workspaces.len() - 1);
                    let ws_id = self.workspaces[idx].id.clone();
                    let ws_slug = self.workspaces[idx].slug.clone();
                    match self
                        .client
                        .create_repo(&ws_id, &name_s, desc_opt.as_deref(), priv_)
                        .await
                    {
                        Ok(repo) => {
                            self.status = format!("created {}/{}", ws_slug, repo.name);
                            self.refresh_repos().await?;
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                            self.modal = modal;
                        }
                    }
                }
                KeyCode::Char(c) => {
                    match field {
                        RepoField::Name => {
                            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                                name.push(c);
                            }
                        }
                        RepoField::Description => description.push(c),
                        RepoField::Visibility => {
                            if c == 'p' || c == 'P' {
                                *is_private = !*is_private;
                            }
                        }
                    }
                    self.modal = modal;
                }
                KeyCode::Backspace => {
                    match field {
                        RepoField::Name => {
                            name.pop();
                        }
                        RepoField::Description => {
                            description.pop();
                        }
                        RepoField::Visibility => {}
                    }
                    self.modal = modal;
                }
                _ => {
                    self.modal = modal;
                }
            },
            Modal::ConfirmDeleteRepo => match key {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.status = "delete cancelled".into();
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    if let Some(row) = self.repos.get(self.list_idx).cloned() {
                        match self
                            .client
                            .delete_repo(&row.workspace_id, &row.repo.name)
                            .await
                        {
                            Ok(()) => {
                                self.status =
                                    format!("deleted {}/{}", row.workspace_slug, row.repo.name);
                                self.refresh_repos().await?;
                            }
                            Err(e) => {
                                self.error = Some(e.to_string());
                            }
                        }
                    }
                }
                _ => {
                    self.modal = modal;
                }
            },
        }
        Ok(())
    }

    async fn handle_repos_key(&mut self, key: KeyCode) -> Result<()> {
        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.list_idx > 0 {
                    self.list_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.repos.is_empty() {
                    self.list_idx = (self.list_idx + 1).min(self.repos.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(row) = self.repos.get(self.list_idx) {
                    self.body = format!(
                        "Repository\n\n  {}/{}\n  {}\n\n  private: {}\n  default branch: {}\n  clone path: {}\n  id: {}\n  workspace: {} ({})\n",
                        row.workspace_slug,
                        row.repo.name,
                        row.repo.description.as_deref().unwrap_or("—"),
                        row.repo.is_private,
                        row.repo.default_branch,
                        row.repo.clone_url_path,
                        row.repo.id,
                        row.workspace_name,
                        row.workspace_id,
                    );
                    self.status = format!("{}/{}", row.workspace_slug, row.repo.name);
                }
            }
            KeyCode::Char('n') => {
                if self.workspaces.is_empty() {
                    self.error = Some("no workspace — press w to create one".into());
                } else {
                    self.modal = Modal::CreateRepo {
                        name: String::new(),
                        description: String::new(),
                        is_private: true,
                        field: RepoField::Name,
                    };
                    self.status = format!(
                        "new repo in {} — Tab fields · Enter create · Esc cancel",
                        self.workspaces[self.selected_workspace.min(self.workspaces.len() - 1)].slug
                    );
                }
            }
            KeyCode::Char('w') => {
                self.modal = Modal::CreateWorkspace {
                    name: String::new(),
                    slug: String::new(),
                    field: WsField::Name,
                };
                self.status = "new workspace — name, slug · Enter create · Esc cancel".into();
            }
            KeyCode::Char('d') => {
                if self.repos.is_empty() {
                    self.error = Some("no repo selected".into());
                } else {
                    self.modal = Modal::ConfirmDeleteRepo;
                    if let Some(row) = self.repos.get(self.list_idx) {
                        self.status = format!(
                            "delete {}/{}? y confirm · n/Esc cancel",
                            row.workspace_slug, row.repo.name
                        );
                    }
                }
            }
            KeyCode::Char('[') => {
                if !self.workspaces.is_empty() {
                    if self.selected_workspace == 0 {
                        self.selected_workspace = self.workspaces.len() - 1;
                    } else {
                        self.selected_workspace -= 1;
                    }
                    self.status = format!(
                        "workspace: {}",
                        self.workspaces[self.selected_workspace].slug
                    );
                }
            }
            KeyCode::Char(']') => {
                if !self.workspaces.is_empty() {
                    self.selected_workspace =
                        (self.selected_workspace + 1) % self.workspaces.len();
                    self.status = format!(
                        "workspace: {}",
                        self.workspaces[self.selected_workspace].slug
                    );
                }
            }
            KeyCode::Char('r') => self.refresh_repos().await?,
            // fall through to global nav
            KeyCode::Char('1')
            | KeyCode::Char('2')
            | KeyCode::Char('3')
            | KeyCode::Char('4')
            | KeyCode::Char('5')
            | KeyCode::Char('6')
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('L') => {
                self.handle_nav_key(key).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_login_key(&mut self, key: KeyCode) -> Result<()> {
        match key {
            KeyCode::Tab | KeyCode::Down => {
                self.login_field = self.next_field(true);
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.login_field = self.next_field(false);
            }
            KeyCode::Enter => match self.auth_mode {
                AuthMode::SignIn => self.try_login().await?,
                AuthMode::Register => self.try_register().await?,
            },
            KeyCode::F(2) => self.toggle_auth_mode(),
            KeyCode::Char(c) => match self.login_field {
                LoginField::Handle => self.handle.push(c),
                LoginField::DisplayName => self.display_name.push(c),
                LoginField::Passphrase => self.passphrase.push(c),
                LoginField::Server => self.server.push(c),
            },
            KeyCode::Backspace => match self.login_field {
                LoginField::Handle => {
                    self.handle.pop();
                }
                LoginField::DisplayName => {
                    self.display_name.pop();
                }
                LoginField::Passphrase => {
                    self.passphrase.pop();
                }
                LoginField::Server => {
                    self.server.pop();
                }
            },
            _ => {}
        }
        Ok(())
    }

    fn toggle_auth_mode(&mut self) {
        self.auth_mode = match self.auth_mode {
            AuthMode::SignIn => AuthMode::Register,
            AuthMode::Register => AuthMode::SignIn,
        };
        self.login_field = LoginField::Handle;
        self.error = None;
        self.status = match self.auth_mode {
            AuthMode::SignIn => "sign in · F2 create account".into(),
            AuthMode::Register => "create account · F2 back to sign in".into(),
        };
    }

    fn next_field(&self, forward: bool) -> LoginField {
        let fields: &[LoginField] = match self.auth_mode {
            AuthMode::SignIn => &[
                LoginField::Handle,
                LoginField::Passphrase,
                LoginField::Server,
            ],
            AuthMode::Register => &[
                LoginField::Handle,
                LoginField::DisplayName,
                LoginField::Passphrase,
                LoginField::Server,
            ],
        };
        let idx = fields
            .iter()
            .position(|f| *f == self.login_field)
            .unwrap_or(0);
        if forward {
            fields[(idx + 1) % fields.len()]
        } else {
            fields[(idx + fields.len() - 1) % fields.len()]
        }
    }

    async fn handle_nav_key(&mut self, key: KeyCode) -> Result<()> {
        let items = Self::menu_items();
        match key {
            KeyCode::Char('1') => {
                self.screen = Screen::Home;
                self.menu_idx = 0;
                self.refresh_home().await?;
            }
            KeyCode::Char('2') => {
                self.screen = Screen::Repos;
                self.menu_idx = 1;
                self.refresh_repos().await?;
            }
            KeyCode::Char('3') => {
                self.screen = Screen::Inbox;
                self.menu_idx = 2;
                self.body = "Issues / PR inbox — wire list endpoints as you expand.".into();
            }
            KeyCode::Char('4') => {
                self.screen = Screen::Channels;
                self.menu_idx = 3;
                self.body = "Channels & Rooms — messaging surface.".into();
            }
            KeyCode::Char('5') => {
                self.screen = Screen::Compute;
                self.menu_idx = 4;
                self.body = "TT Cloud instances / nodes.".into();
            }
            KeyCode::Char('6') => {
                self.screen = Screen::Settings;
                self.menu_idx = 5;
                self.body = format!(
                    "Server: {}\nHandle: {}\nToken: {}",
                    self.server,
                    self.handle,
                    if self.client.token.is_some() {
                        "set"
                    } else {
                        "none"
                    }
                );
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.menu_idx > 0 {
                    self.menu_idx -= 1;
                } else {
                    self.menu_idx = items.len() - 1;
                }
                self.screen = items[self.menu_idx].1;
                self.on_screen_enter().await?;
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.menu_idx = (self.menu_idx + 1) % items.len();
                self.screen = items[self.menu_idx].1;
                self.on_screen_enter().await?;
            }
            KeyCode::Char('r') => {
                self.on_screen_enter().await?;
            }
            KeyCode::Char('L') => {
                self.client.token = None;
                self.logged_in = false;
                self.passphrase.clear();
                self.repos.clear();
                self.workspaces.clear();
                save_config(&SavedConfig {
                    server: self.server.clone(),
                    token: None,
                    handle: Some(self.handle.clone()),
                });
                self.screen = Screen::Login;
                self.status = "signed out".into();
            }
            _ => {}
        }
        Ok(())
    }

    async fn on_screen_enter(&mut self) -> Result<()> {
        match self.screen {
            Screen::Home => self.refresh_home().await?,
            Screen::Repos => self.refresh_repos().await?,
            Screen::Settings => {
                self.body = format!(
                    "Server: {}\nHandle: {}\nLogged in: {}",
                    self.server, self.handle, self.logged_in
                );
            }
            _ => {}
        }
        Ok(())
    }

    async fn try_login(&mut self) -> Result<()> {
        self.client = Client::new(&self.server);
        self.status = "signing in…".into();
        match self.client.login(&self.handle, &self.passphrase).await {
            Ok(res) => {
                self.finish_auth(res.session_token, res.handle, res.display_name)
                    .await?;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.status = "login failed".into();
            }
        }
        Ok(())
    }

    async fn try_register(&mut self) -> Result<()> {
        let display = if self.display_name.trim().is_empty() {
            self.handle.clone()
        } else {
            self.display_name.trim().to_string()
        };
        if self.handle.trim().is_empty() || self.passphrase.is_empty() {
            self.error = Some("handle and passphrase required".into());
            return Ok(());
        }
        self.client = Client::new(&self.server);
        self.status = "creating account…".into();
        match self
            .client
            .register(self.handle.trim(), &display, &self.passphrase)
            .await
        {
            Ok(res) => {
                self.finish_auth(res.session_token, res.handle, res.display_name)
                    .await?;
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.status = "register failed".into();
            }
        }
        Ok(())
    }

    async fn finish_auth(
        &mut self,
        token: String,
        handle: String,
        display_name: String,
    ) -> Result<()> {
        self.client.token = Some(token);
        self.handle = handle;
        self.logged_in = true;
        self.passphrase.clear();
        self.display_name.clear();
        save_config(&SavedConfig {
            server: self.server.clone(),
            token: self.client.token.clone(),
            handle: Some(self.handle.clone()),
        });
        self.screen = Screen::Home;
        self.status = format!("welcome, {display_name}");
        self.refresh_home().await?;
        Ok(())
    }

    async fn refresh_home(&mut self) -> Result<()> {
        match self.client.health().await {
            Ok(h) => {
                self.body = format!(
                    "Turing Tarpit\n\nAPI: {}\n\nShortcuts:\n  1 Home  2 Repos  3 Inbox  4 Channels  5 Compute  6 Settings\n  h/l tabs   r refresh   L logout   q back/quit   Ctrl-C exit\n",
                    h
                );
                self.status = "connected".into();
            }
            Err(e) => {
                self.body = format!("Cannot reach server at {}\n{e}", self.server);
                self.status = "offline".into();
                self.error = Some(e.to_string());
            }
        }
        Ok(())
    }

    pub async fn refresh_repos(&mut self) -> Result<()> {
        self.status = "loading repos…".into();
        match self.client.list_workspaces().await {
            Ok(ws) => {
                self.workspaces = ws;
                let mut rows = Vec::new();
                for w in &self.workspaces {
                    match self.client.list_repos(&w.id).await {
                        Ok(repos) => {
                            for repo in repos {
                                rows.push(RepoRow {
                                    workspace_id: w.id.clone(),
                                    workspace_slug: w.slug.clone(),
                                    workspace_name: w.name.clone(),
                                    repo,
                                });
                            }
                        }
                        Err(e) => {
                            self.error = Some(format!("repos for {}: {e}", w.slug));
                        }
                    }
                }
                rows.sort_by(|a, b| {
                    (&a.workspace_slug, &a.repo.name).cmp(&(&b.workspace_slug, &b.repo.name))
                });
                self.repos = rows;
                if self.list_idx >= self.repos.len() && !self.repos.is_empty() {
                    self.list_idx = self.repos.len() - 1;
                }
                if self.repos.is_empty() {
                    self.body = if self.workspaces.is_empty() {
                        "No workspaces yet.\n\n  w  create workspace\n  n  create repo (after workspace)\n  r  refresh".into()
                    } else {
                        format!(
                            "Workspace(s) ready, no repositories.\n\n  [{}] {}\n\n  n  new repo · w  new workspace\n  [ ] switch workspace · r refresh",
                            self.workspaces[self.selected_workspace.min(self.workspaces.len()-1)].slug,
                            self.workspaces[self.selected_workspace.min(self.workspaces.len()-1)].name,
                        )
                    };
                } else {
                    // body shows detail of selection; list is drawn in ui
                    if let Some(row) = self.repos.get(self.list_idx) {
                        self.body = format!(
                            "{}/{}\n{}",
                            row.workspace_slug,
                            row.repo.name,
                            row.repo.description.as_deref().unwrap_or(""),
                        );
                    }
                }
                self.status = format!(
                    "{} repo(s) · {} workspace(s)",
                    self.repos.len(),
                    self.workspaces.len()
                );
            }
            Err(e) => {
                self.repos.clear();
                self.workspaces.clear();
                self.body = format!("Failed to list workspaces:\n{e}");
                self.status = "error".into();
                self.error = Some(e.to_string());
            }
        }
        Ok(())
    }
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == ' ' || c == '-' || c == '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn config_path() -> std::path::PathBuf {

    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".config")
        .join("turing-tarpit")
        .join("tui.toml")
}

fn load_config() -> SavedConfig {
    let path = config_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &SavedConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = toml::to_string_pretty(cfg) {
        let _ = std::fs::write(path, s);
    }
}
