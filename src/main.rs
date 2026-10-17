#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
#![allow(dead_code)]

mod annots;
mod i18n_de;
mod i18n_es;
mod i18n_it;
mod i18n_pt;
mod i18n_ru;
mod dlg_common;
mod dlg_doc;
mod dlg_pages;
mod updater;
mod rulers;
mod panels_doc;
mod app;
mod app_cmd;
mod app_ext;
mod app_misc;
mod app_ui;
mod autosave;
mod deskew;
mod dialogs;
mod edit;
mod edit_text;
mod export;
mod fonts;
mod forms;
mod geom;
mod i18n;
mod i18n_table;
mod icons;
mod layout;
mod logx;
mod model;
mod pageops;
mod panels;
mod pathdata;
mod pdfedit;
mod pdfedit_annot;
mod pdfedit_doc;
mod pdfedit_forms;
mod pdfedit_pages;
mod pdfx;
mod prefs;
mod props;
mod render;
mod samples;
mod search;
mod sign;
mod signature_pad;
mod sys;
mod tab;
mod textgroup;
mod textsel;
mod tools;
mod view;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    logx::install_panic_hook();
    logx::info(format!("starting RustPDF {}", env!("CARGO_PKG_VERSION")));
    let pdfium = match pdfx::load_pdfium() {
        Ok(p) => p,
        Err(e) => {
            logx::error(format!("{e}"));
            rfd::MessageDialog::new().set_title("RustPDF").set_description(format!("{e}")).show();
            std::process::exit(1);
        }
    };
    let files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).filter(|p| p.exists()).collect();
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1360.0, 880.0]).with_min_inner_size([640.0, 420.0]).with_title("RustPDF").with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("RustPDF", opts, Box::new(move |_cc| Ok(Box::new(app::App::new(pdfium, files)))))
}
