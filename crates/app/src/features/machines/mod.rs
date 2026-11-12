//! Machines: discover the local Herdr Core's registered remote machines,
//! inspect persistent terminal addresses, and open existing/new shell panels.

mod model;
mod view;

use gpui_kit::*;

pub enum MachinesEvent {
    Close,
    /// Attach the existing Herdr terminal address in a new GPUI tab.
    OpenTerminal(String),
    /// Open an ordinary native PTY when the local machine is selected
    /// and the default backend is 'local'.
    NewLocalTerminal,
}

pub struct MachinesView {
    snapshot: Option<model::Snapshot>,
    loading: bool,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
    _load: Task<()>,
    _action: Task<()>,
}

impl EventEmitter<MachinesEvent> for MachinesView {}

/// Only this computer follows the selected native PTY default. A remote
/// machine always uses Herdr, even in the same GPUI window.
fn opens_native_local_shell(machine: Option<&str>, backend: Option<&str>) -> bool {
    machine.is_none() && backend.is_some_and(|v| v.eq_ignore_ascii_case("local"))
}

#[cfg(test)]
mod mixed_backend_tests {
    use super::opens_native_local_shell;

    #[test]
    fn native_local_machine_uses_native_pty() {
        assert!(opens_native_local_shell(None, Some("local")));
        assert!(opens_native_local_shell(None, Some("LOCAL")));
        assert!(!opens_native_local_shell(None, Some("herdr")));
    }

    #[test]
    fn remote_machine_always_uses_herdr() {
        assert!(!opens_native_local_shell(Some("javipi"), Some("local")));
        assert!(!opens_native_local_shell(Some("javipi"), Some("herdr")));
        assert!(!opens_native_local_shell(Some("javipi"), None));
    }
}

impl MachinesView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            snapshot: None,
            loading: false,
            busy: false,
            error: None,
            info: None,
            _load: Task::ready(()),
            _action: Task::ready(()),
        };
        this.refresh(cx);
        this
    }

    /// Background discovery must never block UI painting or key handling.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.loading = true;
        self.error = None;
        let job = cx.background_executor().spawn(async { model::discover() });
        self._load = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => this.snapshot = Some(snapshot),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Open exactly this address; no terminal.run and no process duplication.
    fn open_terminal(&mut self, address: String, cx: &mut Context<Self>) {
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.terminals.iter().any(|t| t.address == address && t.live())
        }) {
            cx.emit(MachinesEvent::OpenTerminal(address));
        }
    }

    /// New shell on this PC follows the local backend; remote machines always
    /// use Herdr even when local PTYs are the default for ordinary tabs.
    fn new_shell(&mut self, machine: Option<String>, cx: &mut Context<Self>) {
        if self.busy { return; }
        let backend = std::env::var("GENTLE_SHELL_TERMINAL_BACKEND").ok();
        if opens_native_local_shell(machine.as_deref(), backend.as_deref()) {
            cx.emit(MachinesEvent::NewLocalTerminal);
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(match &machine {
            Some(name) => format!("Starting a shell on {name}..."),
            None => "Starting a local Herdr shell...".into(),
        });
        let job = cx.background_executor().spawn(async move {
            model::create_shell(machine.as_deref())
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(address) => cx.emit(MachinesEvent::OpenTerminal(address)),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Explicitly request a machine connection; do not SSH from GPUI itself.
    fn connect_machine(&mut self, name: String, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.busy = true;
        self.error = None;
        self.info = Some(format!("Connecting to {name}..."));
        let job = cx.background_executor().spawn(async move {
            model::connect_machine(&name)
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(()) => {
                        this.info = Some("Machine connection requested".into());
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }
}
