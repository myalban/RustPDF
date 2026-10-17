//! Toolbar tools, default styles and commands (menus, shortcuts, palette).

use crate::icons;
use eframe::egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Line,
    Arrow,
    Polygon,
    Freehand,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tool {
    Select,
    Hand,
    /// Text selection (copy, highlight…).
    TextSelect,
    AddText,
    AddImage,
    Shape(ShapeKind),
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Note,
    Stamp,
    Redact,
    Measure,
    Link,
    ExportZone,
    Signature,
    /// Form field creation.
    FormText,
    FormCheck,
    FormRadio,
    FormCombo,
}

impl Tool {
    pub fn label_key(self) -> &'static str {
        match self {
            Tool::Select => "tool_select",
            Tool::Hand => "tool_hand",
            Tool::TextSelect => "tool_textselect",
            Tool::AddText => "tool_addtext",
            Tool::AddImage => "tool_addimage",
            Tool::Shape(ShapeKind::Rect) => "tool_rect",
            Tool::Shape(ShapeKind::Ellipse) => "tool_ellipse",
            Tool::Shape(ShapeKind::Line) => "tool_line",
            Tool::Shape(ShapeKind::Arrow) => "tool_arrow",
            Tool::Shape(ShapeKind::Polygon) => "tool_polygon",
            Tool::Shape(ShapeKind::Freehand) => "tool_freehand",
            Tool::Highlight => "tool_highlight",
            Tool::Underline => "tool_underline",
            Tool::StrikeOut => "tool_strikeout",
            Tool::Squiggly => "tool_squiggly",
            Tool::Note => "tool_note",
            Tool::Stamp => "tool_stamp",
            Tool::Redact => "tool_redact",
            Tool::Measure => "tool_measure",
            Tool::Link => "tool_link",
            Tool::ExportZone => "tool_exportzone",
            Tool::Signature => "tool_signature",
            Tool::FormText => "tool_formtext",
            Tool::FormCheck => "tool_formcheck",
            Tool::FormRadio => "tool_formradio",
            Tool::FormCombo => "tool_formcombo",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Select => icons::CURSOR,
            Tool::Hand => icons::HAND,
            Tool::TextSelect => icons::TEXT_T,
            Tool::AddText => icons::TEXTBOX,
            Tool::AddImage => icons::IMAGE,
            Tool::Shape(ShapeKind::Rect) => icons::RECTANGLE,
            Tool::Shape(ShapeKind::Ellipse) => icons::CIRCLE,
            Tool::Shape(ShapeKind::Line) => icons::LINE_SEGMENT,
            Tool::Shape(ShapeKind::Arrow) => icons::ARROW_UP_RIGHT,
            Tool::Shape(ShapeKind::Polygon) => icons::POLYGON,
            Tool::Shape(ShapeKind::Freehand) => icons::PEN_NIB,
            Tool::Highlight => icons::HIGHLIGHTER,
            Tool::Underline => icons::TEXT_UNDERLINE,
            Tool::StrikeOut => icons::TEXT_STRIKETHROUGH,
            Tool::Squiggly => icons::WAVE_SINE,
            Tool::Note => icons::CHAT_TEXT,
            Tool::Stamp => icons::STAMP,
            Tool::Redact => icons::SELECTION_SLASH,
            Tool::Measure => icons::RULER,
            Tool::Link => icons::LINK,
            Tool::ExportZone => icons::CROP,
            Tool::Signature => icons::SIGNATURE,
            Tool::FormText => icons::TEXT_AA,
            Tool::FormCheck => icons::CHECK_SQUARE,
            Tool::FormRadio => icons::RADIO_BUTTON,
            Tool::FormCombo => icons::CARET_CIRCLE_DOWN,
        }
    }

    /// Tools that work on text selected with the mouse.
    pub fn is_markup(self) -> bool {
        matches!(self, Tool::Highlight | Tool::Underline | Tool::StrikeOut | Tool::Squiggly)
    }
}

/// Application commands: menus, toolbar, shortcuts, command palette, context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Cmd {
    // file
    Open,
    OpenUnprotected,
    OpenRecent(usize),
    Save,
    SaveAs,
    CloseTab,
    NextTab,
    PrevTab,
    Quit,
    Export,
    // edit
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    PasteImage,
    Delete,
    SelectAll,
    Duplicate,
    Find,
    FindNext,
    FindPrev,
    Replace,
    // objects
    Group,
    Ungroup,
    BringFront,
    SendBack,
    BringForward,
    SendBackward,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignTop,
    AlignMiddle,
    AlignBottom,
    DistributeH,
    DistributeV,
    Rotate90,
    RotateMinus90,
    FlipH,
    FlipV,
    // view
    ZoomIn,
    ZoomOut,
    Zoom100,
    FitWidth,
    FitPage,
    ViewSingle,
    ViewContinuous,
    ViewTwo,
    ViewTwoContinuous,
    ToggleCover,
    Fullscreen,
    Presentation,
    ToggleSidePanel,
    ToggleProps,
    ToggleRulers,
    ToggleNight,
    ReadingMode,
    ToggleBoxes,
    ToggleParagraphs,
    // navigation
    FirstPage,
    LastPage,
    NextPage,
    PrevPage,
    GotoPage,
    // pages
    RotatePage,
    DeletePage,
    MovePageUp,
    MovePageDown,
    InsertBlank,
    InsertFromFile,
    ExtractPage,
    SplitDoc,
    MergeFiles,
    CropPages,
    PageSize,
    PageNumbers,
    HeaderFooter,
    Watermark,
    RotatePages,
    DeletePages,
    Deskew,
    // document
    Properties,
    Security,
    CleanMetadata,
    Attachments,
    Bookmarks,
    Sign,
    VerifySignatures,
    FlattenForm,
    // tools
    SetTool(Tool),
    ApplyRedactions,
    // app
    Preferences,
    CommandPalette,
    About,
    CheckUpdates,
    OpenLog,
    RepairDoc,
}

impl Cmd {
    pub fn label_key(self) -> &'static str {
        match self {
            Cmd::Open => "cmd_open",
            Cmd::OpenUnprotected => "cmd_open_unprotected",
            Cmd::OpenRecent(_) => "cmd_open_recent",
            Cmd::Save => "cmd_save",
            Cmd::SaveAs => "cmd_save_as",
            Cmd::CloseTab => "cmd_close_tab",
            Cmd::NextTab => "cmd_next_tab",
            Cmd::PrevTab => "cmd_prev_tab",
            Cmd::Quit => "cmd_quit",
            Cmd::Export => "cmd_export",
            Cmd::Undo => "cmd_undo",
            Cmd::Redo => "cmd_redo",
            Cmd::Copy => "cmd_copy",
            Cmd::Cut => "cmd_cut",
            Cmd::Paste => "cmd_paste",
            Cmd::PasteImage => "cmd_paste_image",
            Cmd::Delete => "cmd_delete",
            Cmd::SelectAll => "cmd_select_all",
            Cmd::Duplicate => "cmd_duplicate",
            Cmd::Find => "cmd_find",
            Cmd::FindNext => "cmd_find_next",
            Cmd::FindPrev => "cmd_find_prev",
            Cmd::Replace => "cmd_replace",
            Cmd::Group => "cmd_group",
            Cmd::Ungroup => "cmd_ungroup",
            Cmd::BringFront => "cmd_bring_front",
            Cmd::SendBack => "cmd_send_back",
            Cmd::BringForward => "cmd_bring_forward",
            Cmd::SendBackward => "cmd_send_backward",
            Cmd::AlignLeft => "cmd_align_left",
            Cmd::AlignCenter => "cmd_align_center",
            Cmd::AlignRight => "cmd_align_right",
            Cmd::AlignTop => "cmd_align_top",
            Cmd::AlignMiddle => "cmd_align_middle",
            Cmd::AlignBottom => "cmd_align_bottom",
            Cmd::DistributeH => "cmd_distribute_h",
            Cmd::DistributeV => "cmd_distribute_v",
            Cmd::Rotate90 => "cmd_rotate90",
            Cmd::RotateMinus90 => "cmd_rotate_minus90",
            Cmd::FlipH => "cmd_flip_h",
            Cmd::FlipV => "cmd_flip_v",
            Cmd::ZoomIn => "cmd_zoom_in",
            Cmd::ZoomOut => "cmd_zoom_out",
            Cmd::Zoom100 => "cmd_zoom_100",
            Cmd::FitWidth => "cmd_fit_width",
            Cmd::FitPage => "cmd_fit_page",
            Cmd::ViewSingle => "cmd_view_single",
            Cmd::ViewContinuous => "cmd_view_continuous",
            Cmd::ViewTwo => "cmd_view_two",
            Cmd::ViewTwoContinuous => "cmd_view_two_continuous",
            Cmd::ToggleCover => "cmd_toggle_cover",
            Cmd::Fullscreen => "cmd_fullscreen",
            Cmd::Presentation => "cmd_presentation",
            Cmd::ToggleSidePanel => "cmd_toggle_side",
            Cmd::ToggleProps => "cmd_toggle_props",
            Cmd::ToggleRulers => "cmd_toggle_rulers",
            Cmd::ToggleNight => "cmd_toggle_night",
            Cmd::ReadingMode => "cmd_reading_mode",
            Cmd::ToggleBoxes => "cmd_toggle_boxes",
            Cmd::ToggleParagraphs => "cmd_toggle_paragraphs",
            Cmd::FirstPage => "cmd_first_page",
            Cmd::LastPage => "cmd_last_page",
            Cmd::NextPage => "cmd_next_page",
            Cmd::PrevPage => "cmd_prev_page",
            Cmd::GotoPage => "cmd_goto_page",
            Cmd::RotatePage => "cmd_rotate_page",
            Cmd::DeletePage => "cmd_delete_page",
            Cmd::MovePageUp => "cmd_move_page_up",
            Cmd::MovePageDown => "cmd_move_page_down",
            Cmd::InsertBlank => "cmd_insert_blank",
            Cmd::InsertFromFile => "cmd_insert_from_file",
            Cmd::ExtractPage => "cmd_extract_page",
            Cmd::SplitDoc => "cmd_split",
            Cmd::MergeFiles => "cmd_merge",
            Cmd::CropPages => "cmd_crop_pages",
            Cmd::PageSize => "cmd_page_size",
            Cmd::PageNumbers => "cmd_page_numbers",
            Cmd::HeaderFooter => "cmd_header_footer",
            Cmd::Watermark => "cmd_watermark",
            Cmd::RotatePages => "cmd_rotate_pages",
            Cmd::DeletePages => "cmd_delete_pages",
            Cmd::Deskew => "cmd_deskew",
            Cmd::Properties => "cmd_properties",
            Cmd::Security => "cmd_security",
            Cmd::CleanMetadata => "cmd_clean_metadata",
            Cmd::Attachments => "cmd_attachments",
            Cmd::Bookmarks => "cmd_bookmarks",
            Cmd::Sign => "cmd_sign",
            Cmd::VerifySignatures => "cmd_verify_signatures",
            Cmd::FlattenForm => "cmd_flatten_form",
            Cmd::SetTool(t) => t.label_key(),
            Cmd::ApplyRedactions => "cmd_apply_redactions",
            Cmd::Preferences => "cmd_preferences",
            Cmd::CommandPalette => "cmd_palette",
            Cmd::About => "cmd_about",
            Cmd::CheckUpdates => "cmd_check_updates",
            Cmd::OpenLog => "cmd_open_log",
            Cmd::RepairDoc => "cmd_repair",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Cmd::Open | Cmd::OpenRecent(_) => icons::FOLDER_OPEN,
            Cmd::OpenUnprotected => icons::PENCIL_SIMPLE,
            Cmd::Save | Cmd::SaveAs => icons::FLOPPY_DISK,
            Cmd::Undo => icons::ARROW_COUNTER_CLOCKWISE,
            Cmd::Redo => icons::ARROW_CLOCKWISE,
            Cmd::Copy => icons::COPY,
            Cmd::Cut => icons::SCISSORS,
            Cmd::Paste | Cmd::PasteImage => icons::CLIPBOARD,
            Cmd::Delete | Cmd::DeletePage | Cmd::DeletePages => icons::TRASH,
            Cmd::Find | Cmd::FindNext | Cmd::FindPrev => icons::MAGNIFYING_GLASS,
            Cmd::ZoomIn => icons::MAGNIFYING_GLASS_PLUS,
            Cmd::ZoomOut => icons::MAGNIFYING_GLASS_MINUS,
            Cmd::FitWidth => icons::ARROWS_HORIZONTAL,
            Cmd::FitPage => icons::ARROWS_OUT,
            Cmd::Rotate90 | Cmd::RotatePage | Cmd::RotatePages => icons::ARROW_CLOCKWISE,
            Cmd::RotateMinus90 => icons::ARROW_COUNTER_CLOCKWISE,
            Cmd::Group => icons::SELECTION_PLUS,
            Cmd::Ungroup => icons::SELECTION_SLASH,
            Cmd::BringFront => icons::ARROW_LINE_UP,
            Cmd::BringForward => icons::ARROW_UP,
            Cmd::Duplicate => icons::STACK_PLUS,
            Cmd::SendBack => icons::ARROW_LINE_DOWN,
            Cmd::SendBackward => icons::ARROW_DOWN,
            Cmd::AlignLeft => icons::ALIGN_LEFT,
            Cmd::AlignCenter => icons::ALIGN_CENTER_HORIZONTAL,
            Cmd::AlignRight => icons::ALIGN_RIGHT,
            Cmd::AlignTop => icons::ALIGN_TOP,
            Cmd::AlignMiddle => icons::ALIGN_CENTER_VERTICAL,
            Cmd::AlignBottom => icons::ALIGN_BOTTOM,
            Cmd::FlipH => icons::FLIP_HORIZONTAL,
            Cmd::FlipV => icons::FLIP_VERTICAL,
            Cmd::Preferences => icons::GEAR,
            Cmd::Sign => icons::SIGNATURE,
            Cmd::Security => icons::LOCK,
            Cmd::Attachments => icons::PAPERCLIP,
            Cmd::Bookmarks => icons::BOOKMARK_SIMPLE,
            Cmd::Presentation | Cmd::Fullscreen => icons::PRESENTATION_CHART,
            Cmd::ReadingMode => icons::BOOK_OPEN,
            Cmd::SetTool(t) => t.icon(),
            Cmd::InsertBlank => icons::FILE_PLUS,
            Cmd::Export => icons::EXPORT,
            Cmd::About => icons::INFO,
            Cmd::CommandPalette => icons::COMMAND,
            _ => "",
        }
    }

    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        let c = Modifiers::COMMAND;
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        let sh = Modifiers::SHIFT;
        let n = Modifiers::NONE;
        let k = |m, key| Some(KeyboardShortcut::new(m, key));
        match self {
            Cmd::Open => k(c, Key::O),
            Cmd::OpenUnprotected => k(cs, Key::O),
            Cmd::Save => k(c, Key::S),
            Cmd::SaveAs => k(cs, Key::S),
            Cmd::CloseTab => k(c, Key::W),
            Cmd::NextTab => k(Modifiers::CTRL, Key::Tab),
            Cmd::Undo => k(c, Key::Z),
            Cmd::Redo => k(c, Key::Y),
            Cmd::Copy => k(c, Key::C),
            Cmd::Cut => k(c, Key::X),
            Cmd::Paste => k(c, Key::V),
            Cmd::SelectAll => k(c, Key::A),
            Cmd::Duplicate => k(c, Key::D),
            Cmd::Find => k(c, Key::F),
            Cmd::FindNext => k(n, Key::F3),
            Cmd::FindPrev => k(sh, Key::F3),
            Cmd::Replace => k(c, Key::H),
            Cmd::Group => k(c, Key::G),
            Cmd::Ungroup => k(cs, Key::G),
            Cmd::BringFront => k(cs, Key::CloseBracket),
            Cmd::SendBack => k(cs, Key::OpenBracket),
            Cmd::ZoomIn => k(c, Key::Equals),
            Cmd::ZoomOut => k(c, Key::Minus),
            Cmd::Zoom100 => k(c, Key::Num1),
            Cmd::FitWidth => k(c, Key::Num2),
            Cmd::FitPage => k(c, Key::Num0),
            Cmd::Fullscreen => k(n, Key::F11),
            Cmd::ReadingMode => k(n, Key::F9),
            Cmd::Presentation => k(c, Key::L),
            Cmd::ToggleSidePanel => k(n, Key::F4),
            Cmd::ToggleProps => k(sh, Key::F4),
            Cmd::FirstPage => k(n, Key::Home),
            Cmd::LastPage => k(n, Key::End),
            Cmd::GotoPage => k(cs, Key::N),
            Cmd::CommandPalette => k(cs, Key::P),
            Cmd::Preferences => k(c, Key::Comma),
            Cmd::Properties => k(c, Key::D).filter(|_| false),
            _ => None,
        }
    }

    /// Commands available in the palette (Ctrl+Shift+P).
    pub fn palette_list() -> Vec<Cmd> {
        use Cmd::*;
        let mut v = vec![
            Open, OpenUnprotected, Save, SaveAs, CloseTab, Export, Undo, Redo, Copy, Cut, Paste, PasteImage, Delete, SelectAll, Duplicate, Find, FindNext, FindPrev,
            Replace, Group, Ungroup, BringFront, SendBack, BringForward, SendBackward, AlignLeft, AlignCenter, AlignRight, AlignTop,
            AlignMiddle, AlignBottom, DistributeH, DistributeV, Rotate90, RotateMinus90, FlipH, FlipV, ZoomIn, ZoomOut, Zoom100, FitWidth,
            FitPage, ViewSingle, ViewContinuous, ViewTwo, ViewTwoContinuous, ToggleCover, Fullscreen, Presentation, ToggleSidePanel,
            ToggleProps, ToggleRulers, ToggleNight, ReadingMode, ToggleBoxes, ToggleParagraphs, FirstPage, LastPage, NextPage, PrevPage, GotoPage,
            RotatePage, DeletePage, MovePageUp, MovePageDown, InsertBlank, InsertFromFile, ExtractPage, SplitDoc, MergeFiles, CropPages,
            PageSize, PageNumbers, HeaderFooter, Watermark, RotatePages, DeletePages, Deskew, Properties, Security, CleanMetadata,
            Attachments, Bookmarks, Sign, VerifySignatures, FlattenForm, ApplyRedactions, Preferences, About, CheckUpdates,
            OpenLog, RepairDoc,
        ];
        for t in [
            Tool::Select,
            Tool::Hand,
            Tool::TextSelect,
            Tool::AddText,
            Tool::AddImage,
            Tool::Shape(ShapeKind::Rect),
            Tool::Shape(ShapeKind::Ellipse),
            Tool::Shape(ShapeKind::Line),
            Tool::Shape(ShapeKind::Arrow),
            Tool::Shape(ShapeKind::Polygon),
            Tool::Shape(ShapeKind::Freehand),
            Tool::Highlight,
            Tool::Underline,
            Tool::StrikeOut,
            Tool::Squiggly,
            Tool::Note,
            Tool::Stamp,
            Tool::Redact,
            Tool::Measure,
            Tool::Link,
            Tool::ExportZone,
            Tool::Signature,
            Tool::FormText,
            Tool::FormCheck,
            Tool::FormRadio,
            Tool::FormCombo,
        ] {
            v.push(SetTool(t));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_label_keys_are_translated() {
        use crate::i18n_table::TABLE;
        let mut missing = vec![];
        let mut check = |k: &str| {
            if !TABLE.iter().any(|(key, _, _)| *key == k) {
                missing.push(k.to_string());
            }
        };
        for c in Cmd::palette_list() {
            check(c.label_key());
        }
        for c in [Cmd::OpenRecent(0), Cmd::NextTab, Cmd::PrevTab, Cmd::Quit, Cmd::CheckUpdates, Cmd::About] {
            check(c.label_key());
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "untranslated labels: {missing:?}");
    }

    #[test]
    fn every_command_in_palette_has_a_label_key() {
        for c in Cmd::palette_list() {
            assert!(!c.label_key().is_empty());
        }
    }

    #[test]
    fn shortcuts_are_unique() {
        let mut seen = std::collections::HashMap::new();
        for c in Cmd::palette_list() {
            if let Some(s) = c.shortcut() {
                let key = format!("{:?}", s);
                if let Some(prev) = seen.insert(key.clone(), c) {
                    panic!("shortcut {key} used by {prev:?} and {c:?}");
                }
            }
        }
    }
}

/// Current settings of the drawing and annotation tools.
#[derive(Clone, Debug)]
pub struct ToolStyle {
    pub shape_stroke: [u8; 3],
    pub shape_fill: Option<[u8; 3]>,
    pub shape_width: f32,
    /// 0 = solid, 1 = dashed, 2 = dotted.
    pub shape_dash: u8,
    pub shape_opacity: u8,
    pub markup_color: [u8; 3],
    pub markup_opacity: u8,
    pub text: crate::edit_text::TextStyle,
    pub stamp_text: String,
    pub stamp_color: [u8; 3],
    pub note_text: String,
}

impl Default for ToolStyle {
    fn default() -> Self {
        ToolStyle {
            shape_stroke: [220, 30, 30],
            shape_fill: None,
            shape_width: 2.0,
            shape_dash: 0,
            shape_opacity: 255,
            markup_color: [255, 235, 59],
            markup_opacity: 140,
            text: crate::edit_text::TextStyle::default(),
            stamp_text: "APPROVED".into(),
            stamp_color: [30, 140, 60],
            note_text: String::new(),
        }
    }
}

impl ToolStyle {
    pub fn dash_array(&self) -> Vec<f32> {
        match self.shape_dash {
            1 => vec![self.shape_width * 4.0, self.shape_width * 2.0],
            2 => vec![self.shape_width, self.shape_width * 2.0],
            _ => vec![],
        }
    }
}
