; Inno Setup script (https://jrsoftware.org/isinfo.php) - compile with: iscc installer\rustpdf.iss
; Prerequisite: cargo build --release  (the script copies target\release\rustpdf.exe and pdfium.dll)
; Prerequisite: cargo build --release  (the script copies target
elease
ustpdf.exe and pdfium.dll)
#define AppVer "0.2.0"
[Setup]
AppName=RustPDF
AppVersion={#AppVer}
AppPublisher=Alban DOUSSAU de GUYONNET
DefaultDirName={autopf}\RustPDF
DefaultGroupName=RustPDF
OutputDir=..\dist
OutputBaseFilename=RustPDF-Setup-{#AppVer}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\rustpdf.exe
[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"
[Tasks]
Name: "desktopicon"; Description: "Create a desktop icon"; Flags: unchecked
Name: "assoc"; Description: "Associate .pdf files with RustPDF (open by default)"; Flags: unchecked
[Files]
Source: "..\target\release\rustpdf.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs
[Icons]
Name: "{group}\RustPDF"; Filename: "{app}\rustpdf.exe"
Name: "{autodesktop}\RustPDF"; Filename: "{app}\rustpdf.exe"; Tasks: desktopicon
[Registry]
Root: HKA; Subkey: "Software\Classes\RustPDF.pdf"; ValueType: string; ValueData: "Document PDF"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustPDF.pdf\DefaultIcon"; ValueType: string; ValueData: "{app}\rustpdf.exe,0"
Root: HKA; Subkey: "Software\Classes\RustPDF.pdf\shell\open\command"; ValueType: string; ValueData: """{app}\rustpdf.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "RustPDF.pdf"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
[Run]
Filename: "{app}\rustpdf.exe"; Description: "Launch RustPDF"; Flags: nowait postinstall skipifsilent
