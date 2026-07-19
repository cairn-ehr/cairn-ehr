//! The iced application: two panes via pane_grid (resizable divider = the user
//! splitter), per-pane tab strips, a pinned identity card, and OpenTab routing
//! into the opposite pane. All non-iced decisions delegate to the pure Workspace.
use crate::workspace::{Side, Workspace};
use cairn_gui_data::MockData;
use cairn_gui_manifest::{merge, EffectiveManifest, SiteManifest, UserPrefs};
use cairn_gui_tab::{
    Capabilities, Context, Field, PatientRef, Role, Semantic, SemanticNode, TabId, UserRef,
};
use cairn_gui_tab_demographics::DemographicsTab;
use cairn_gui_tab_note::NoteTab;

use iced::widget::{button, column, container, pane_grid, row, scrollable, text};
use iced::{Element, Length, Task};

#[derive(Debug, Clone)]
pub enum Message {
    Resized(pane_grid::ResizeEvent),
    SelectTab(Side, TabId),
    OpenRef(Side, String), // Side = originating pane, String = target id
}

struct PaneState {
    side: Side,
}

pub struct App {
    ctx: Context,
    // `data` is retained even though `MockData` is read once at boot: a later
    // slice reloads a tab's data on activation (background scheduler), and the
    // read-only mock port is the seam that swaps in for a real node.
    #[allow(dead_code)]
    data: MockData,
    ws: Workspace,
    note: NoteTab,
    demographics: DemographicsTab,
    panes: pane_grid::State<PaneState>,
}

fn default_site() -> SiteManifest {
    SiteManifest {
        offered: vec![TabId("note".into()), TabId("demographics".into())],
        rail: vec![TabId("note".into()), TabId("demographics".into())],
        default_left: TabId("note".into()),
        default_right: TabId("demographics".into()),
    }
}

fn effective() -> EffectiveManifest {
    merge(&default_site(), &UserPrefs::default())
}

impl App {
    pub fn boot() -> (Self, Task<Message>) {
        let ctx = Context {
            patient: Some(PatientRef {
                uuid: "00000000-0000-0000-0000-0000000000aa".into(),
                display_name: "Amina أمينة अमीना 阿明娜".into(),
            }),
            user: UserRef {
                actor_id: "clin-1".into(),
                display_name: "Dr Vega".into(),
            },
            capabilities: Capabilities::clinician_all(),
        };
        let data = MockData::with_fixtures();

        // Eager load for both visible panes' initial tabs (slice 1: both visible).
        let mut note = NoteTab::new();
        note.load(&ctx, &data);
        let mut demographics = DemographicsTab::new();
        demographics.load(&ctx, &data);

        let eff = effective();
        let ws = Workspace::from_manifest(&eff);

        // pane_grid: one vertical split (left | right), ratio taken from the
        // manifest so a returning user's apportioned split is honoured. `split`
        // returns `None` only if `left` is not a valid pane in `panes`, which
        // cannot happen immediately after `State::new` handed it back to us.
        let (mut panes, left) = pane_grid::State::new(PaneState { side: Side::Left });
        let (_right_pane, split) = panes
            .split(
                pane_grid::Axis::Vertical,
                left,
                PaneState { side: Side::Right },
            )
            .expect("freshly created pane is always splittable");
        panes.resize(split, ws.divider_ratio);

        (
            Self {
                ctx,
                data,
                ws,
                note,
                demographics,
                panes,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Resized(ev) => {
                self.panes.resize(ev.split, ev.ratio);
                self.ws.divider_ratio = ev.ratio;
            }
            Message::SelectTab(side, tab) => self.ws.activate(side, &tab),
            Message::OpenRef(from, target_id) => {
                // Cross-reference routing (spec §5): open the referenced view in the
                // OTHER pane, leaving the originating pane (the note) in place. Slice
                // 1 has a single reference target; a real reference registry mapping
                // ids → tabs arrives with the results/imaging tabs.
                let _ = target_id;
                self.ws.open_in_opposite(from, TabId("demographics".into()));
            }
        }
        Task::none()
    }

    fn tab_view(&self, side: Side, active: &TabId) -> Element<'_, Message> {
        // Render the active tab's semantics as labelled controls. Cross-ref buttons
        // (id "open:<x>") emit OpenRef so routing lands in the opposite pane.
        let semantic = match active.0.as_str() {
            "note" => self.note.semantics(&self.ctx),
            "demographics" => self.demographics.semantics(&self.ctx),
            _ => {
                // An unknown TabId must never silently fall through to whatever tab
                // happens to be last in this match (that would render the WRONG
                // CHART — principle 3, paper-parity: a confirmation dialog is not an
                // acceptable substitute for this being loud). Dev/test builds abort
                // via debug_assert; release builds render a visible placeholder so
                // the clinician sees "something is wrong" instead of another
                // patient's data.
                debug_assert!(false, "unknown TabId in tab_view: {}", active.0);
                SemanticNode {
                    title: "Unknown tab".into(),
                    fields: vec![Field {
                        id: "unknown".into(),
                        role: Role::Heading,
                        label: format!("Unknown tab: {}", active.0),
                    }],
                }
            }
        };
        let mut col = column![text(semantic.title).size(18)].spacing(6);
        for f in semantic.fields {
            if let Some(target) = f.id.strip_prefix("open:") {
                let target = target.to_string();
                col = col.push(button(text(f.label)).on_press(Message::OpenRef(side, target)));
            } else {
                col = col.push(text(f.label));
            }
        }
        scrollable(col).height(Length::Fill).into()
    }

    fn pane_content(&self, side: Side) -> Element<'_, Message> {
        // A tab strip over the active tab body.
        let mut strip = row![].spacing(4);
        for t in self.ws.tabs(side) {
            let title = match t.0.as_str() {
                "note" => self.note.title(),
                "demographics" => self.demographics.title(),
                other => other.to_string(),
            };
            // Active-tab visual styling is later polish; the pane already tracks its
            // active tab in Workspace. Slice 1 renders the strip functionally.
            strip = strip.push(button(text(title)).on_press(Message::SelectTab(side, t.clone())));
        }
        let body = self.tab_view(side, self.ws.active(side));
        column![strip, body].spacing(8).padding(8).into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        // Band 1+2: pinned identity card (persistent safety zone).
        let name = self
            .ctx
            .patient
            .as_ref()
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        let identity = container(text(format!("Patient: {name}")).size(16)).padding(10);

        // Band 4: two-pane workspace with a draggable divider (the user-apportioned
        // splitter — no fixed pixel widths, spec's no-fixed-pixels rule).
        let grid = pane_grid(&self.panes, |_id, state, _is_maximized| {
            pane_grid::Content::new(self.pane_content(state.side))
        })
        .on_resize(10, Message::Resized)
        .width(Length::Fill)
        .height(Length::Fill);

        column![identity, grid].into()
    }
}

/// Entry point called by `main`: boots the iced application with the standard
/// boot/update/view triple. No signing/writing ever happens here — the shell is
/// read-only against the mock port for slice 1 (spec's UI-never-signs rule).
pub fn run_gui() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title("Cairn — clinician chart")
        .run()
}
