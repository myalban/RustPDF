# Security policy

## Reporting a vulnerability
Please **do not open a public issue** for a security problem. Use GitHub's private reporting instead:
**Security → Report a vulnerability** on the repository page. I will answer as soon as I can and credit you in the fix unless you prefer otherwise.

Useful details: the RustPDF version, your OS, and a PDF (or steps) that reproduces the problem.

## What RustPDF does and does not do
- PDFs are untrusted input. They are parsed by PDFium and by `lopdf`; PDF JavaScript is **never executed**.
- Links inside a PDF are opened only if they are `http`, `https` or `mailto`; anything else (`file:`, `javascript:`, network shares…) is refused.
- No document content is sent anywhere. The only network access is an optional once-a-day request to GitHub to look for a newer release (Preferences → *Check for updates*); nothing is downloaded or installed automatically.
- Documents with unsaved changes are copied to the application data folder for crash recovery (`%APPDATA%\RustPDF\autosave`, `~/.config/rustpdf/autosave`, `~/Library/Application Support/RustPDF/autosave`) and removed once the document is saved or the recovery is dismissed. Disable it in Preferences if the documents are sensitive.
- Digital signatures use the RustCrypto `rsa` crate, which has a known timing side channel (RUSTSEC-2023-0071, no fix available yet) that matters for servers, not for a desktop app signing locally with its own key. They do not validate the certificate chain against a trust store: a valid mark means "unmodified since signed by this key", not "signed by a trusted identity".
- Redaction removes the text, the image pixels and the annotations under the marked area. Always re-open the saved file and check before sharing it.

## Supported versions
Only the latest release receives fixes.
