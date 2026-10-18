# Contributing to RustPDF

Thanks for wanting to help! RustPDF is a young project and every kind of contribution is welcome:
bug reports, translations, documentation, new features, or just trying it on your own PDFs and telling us what breaks.

## Quick start
```
git clone https://github.com/myalban/rustpdf.git
cd rustpdf
sh scripts/fetch-pdfium.sh        # Windows: scripts\fetch-pdfium.ps1
cargo run --release -- samples/demo.pdf
cargo test
```
Linux needs a few system libraries, see the README.

## Where things live
| Area | Files |
|------|-------|
| Window, menus, toolbar, shortcuts | `src/app*.rs`, `src/tools.rs` |
| Page view (tiles, selection, handles, mouse) | `src/view.rs`, `src/render.rs`, `src/layout.rs` |
| Panels and dialogs | `src/panels*.rs`, `src/props.rs`, `src/dialogs.rs`, `src/dlg_*.rs` |
| Editing engine (text, objects, annotations, forms) | `src/edit*.rs`, `src/textgroup.rs`, `src/annots.rs`, `src/forms.rs`, `src/pathdata.rs` |
| File-level operations (`lopdf`) | `src/pdfedit*.rs`, `src/pageops.rs` |
| Signatures and security | `src/sign.rs`, `src/pdfedit_doc.rs` |
| Translations | `src/i18n*.rs` |

The architecture notes worth knowing before you dive in:
- Everything is stored in PDF page space; `PageGeom` converts to the on-screen view (it handles rotation and offset crop boxes).
- Edits go through `Tab::edit_page` (PDFium objects) or `Tab::replace_with_bytes` (`lopdf` operations); both create an undo snapshot.
- PDFium is not thread-safe: tests run one at a time (`.cargo/config.toml`) and rendering is incremental on the UI thread.
- Several PDFium / pdfium-render pitfalls are documented in comments next to the code that works around them.

## Before you open a pull request
- `cargo test` passes and `cargo build` produces no warnings.
- New UI text goes through `t("key")` and gets a key in `src/i18n_en.rs` (English, the default) and in every `src/i18n_xx.rs`. The tests tell you if a key is missing.
- Comments and identifiers are in English.
- Keep changes focused; a short description of *why* helps the review.

## Adding or improving a language
Copy `src/i18n_fr.rs` to `src/i18n_xx.rs`, translate the values (keep the placeholders such as `{n}` and `{date}`), then register it in `src/main.rs`, `Lang` (`src/prefs.rs`) and `extra_tables` (`src/i18n.rs`). `cargo test i18n` verifies that nothing is missing.

## Good first issues
- Review a translation (they were written without a native review).
- Test on your platform (Linux distributions, macOS versions) and report what differs.
- Open a PDF that behaves badly, attach it (if shareable) and describe what you expected.
- Improve the demo document or the screenshots in `docs/images`.

## Reporting bugs
Use the bug report template. Please include your OS, the RustPDF version (Help → About), the steps to reproduce, and the log file (`Preferences → Data folder`, file `rustpdf.log`; crash reports are in the same folder).
