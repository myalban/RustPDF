# RustPDF

Desktop PDF editor written in Rust (egui + PDFium + lopdf), inspired by Adobe Acrobat. Windows 10/11.

## Features
- **Reading**: continuous scroll, single/two pages, cover, full screen, presentation, reading mode, wheel zoom centered on the cursor, tiled rendering with cache, tabs, rulers and coordinates (pt/mm/cm/in), night mode, last position remembered per file.
- **Text**: in-place editing, merged paragraphs with reflow, bold/italic/underline/strikethrough, alignment/justification, letter/line spacing, embedded system fonts, font replacement, find/replace, selection and copy.
- **Objects**: multi-selection, groups, copy/paste (across pages), stacking order, align/distribute, rotate/flip, image crop/brightness/compression, shapes, path points, numeric position/size, PNG/JPEG/SVG export, image extraction.
- **Annotations**: highlight/underline/strikeout/squiggly, notes, stamps, permanent redaction, comments panel with replies.
- **Forms**: filling, lists, field creation, Tab navigation, simple calculations, flattening.
- **Security**: handwritten signature, PKCS#7 digital signature (RSA, .pfx or self-signed certificate) and verification, AES-256 encryption with permissions, metadata cleaning.
- **Pages/document**: drag-and-drop reordering, insert/extract/split/merge, crop, page size, page numbers, headers/footers, watermark, deskew, bookmarks, links, attachments, layers.
- **Application**: undo history, autosave and crash recovery, command palette, preferences, log and crash reports.
- **Languages**: English, French, Spanish, German, Italian, Portuguese and Russian. The interface starts in the language of the computer (Preferences → Language to change it); a language without a translation falls back to English.

## Known limitations
- No rendering threads (PDFium is not thread-safe): incremental rendering with a time budget per frame.
- Undo/redo uses full snapshots (adjustable memory cap).
- Signatures: RSA only, no trust-chain validation.
- Text in subset fonts: replaced by a system font when glyphs are missing.
- Automatic updates need an address that hosts a JSON file `{"version","url","notes"}` (Preferences).

## Build
```
cargo build --release          # needs pdfium.dll next to the executable (bblanchon/pdfium-binaries)
cargo test                     # sequential tests (PDFium is not thread-safe)
installer\build-portable.ps1   # portable zip
iscc installer\rustpdf.iss     # installer (Inno Setup 6)
```

## Adding a language
Copy `src/i18n_es.rs` to `src/i18n_xx.rs`, translate the texts, register the module in `src/main.rs`, add the language to `Lang` in `src/prefs.rs` and to `extra_tables` in `src/i18n.rs`. The tests check that every key is translated.

License: MIT; PDFium: see `licenses/`.
