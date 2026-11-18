//! A single native directory chooser for every new agent entry point.
//! GPUI supplies platform dialogs on Windows, Linux and macOS.

use super::*;

pub(super) enum AgentDestination {
    Tab { prompt: Option<String> },
    Split(SplitAxis),
}

impl Workspace {
    pub(super) fn pick_agent_folder(
        &mut self,
        destination: AgentDestination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open agent in folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            // Canceling or dismissing the OS picker must not create an agent.
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(folder) = paths.pop() else {
                return;
            };
            let _ = this.update_in(cx, move |this, window, cx| {
                if !folder.is_dir() {
                    window.push_notification(
                        Notification::info("The selected folder is no longer available."),
                        cx,
                    );
                    return;
                }
                match destination {
                    AgentDestination::Tab { prompt } => {
                        this.open_tab(PaneKind::Agent, folder, prompt, None, window, cx);
                    }
                    AgentDestination::Split(axis) => {
                        this.split_at(axis, PaneKind::Agent, folder, window, cx);
                    }
                }
            });
        })
        .detach();
    }
}
