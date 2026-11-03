; Concat's Windows installer.
;
; Built by build-app.yml with Inno Setup from the staged folder the .msi
; is also made of, so the two ship the same files; this is the one a
; person double-clicks, and it puts Concat in the Start menu and can take
; it out again. Everything it needs is handed in on the command line:
;
;   iscc /DVersion=0.2.8 /DArch=x64compatible /DSuffix=x86_64 ^
;        /DStage=C:\...\stage\Concat-0.2.8-windows-x86_64 /DOut=C:\...\stage ^
;        assets\windows\concat.iss
;
; Arch is Inno's own word for the machine: x64compatible for the x86_64
; build (which also installs on ARM PCs, under emulation), arm64 for the
; native one. Suffix is the bundle's word for it, which names the file.

#ifndef Version
  #error Version is required
#endif
#ifndef Arch
  #error Arch is required: x64compatible or arm64
#endif
#ifndef Suffix
  #error Suffix is required: x86_64 or aarch64
#endif
#ifndef Stage
  #error Stage is required: the staged folder to install
#endif
#ifndef Out
  #define Out "."
#endif

[Setup]
; One id for the life of the product, so an install over an older one is
; an upgrade and not a second copy.
; A new id for 128 pixeles video, so it never upgrades or removes Concat.
AppId={{3F6A2D81-128B-4C7E-9A50-128F1E0B7D29}
AppName=128 pixeles video
AppVersion={#Version}
AppVerName=128 pixeles video {#Version}
AppPublisher=128 Pixeles
AppPublisherURL=https://github.com/camilofilho/128pixelesvideo
AppSupportURL=https://github.com/camilofilho/128pixelesvideo/issues
AppUpdatesURL=https://github.com/camilofilho/128pixelesvideo/releases
DefaultDirName={autopf}\128 pixeles video
DefaultGroupName=128 pixeles video
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
OutputDir={#Out}
OutputBaseFilename=128 pixeles video-{#Version}-setup
SetupIconFile=..\icons\concat.ico
UninstallDisplayIcon={app}\128 pixeles video.ico
UninstallDisplayName=128 pixeles video
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
; For the user alone unless they ask for the machine: no prompt for an
; administrator to install a video editor into one's own account.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; Follow Windows' display language when the setup speaks it, and ask only
; when it does not.
ShowLanguageDialog=auto

[Languages]
; Every language the app ships. Inno Setup carries the official
; translations. The ones it keeps as unofficial - Chinese, Persian,
; Croatian, Bengali, Hindi, Indonesian, Urdu, Vietnamese - and the ones
; made official lately - Korean, Arabic and Thai, which an older compiler
; lacks - are vendored in languages/ from the Inno Setup 6.7.1 tree
; (https://github.com/jrsoftware/issrc/tree/is-6_7_1/Files/Languages),
; verbatim, so the setup offers them whatever the compiler on the build
; machine ships (#277). Some are in an older format, which the compiler
; still reads with the newer messages in English and a warning for each:
; Traditional Chinese 6.1, Urdu 6.1, Indonesian 6.4, and Bengali and Hindi
; 5.5.3, whose obsolete title and copyright font sizes it ignores.
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "chinesetraditional"; MessagesFile: "languages\ChineseTraditional.isl"
Name: "korean"; MessagesFile: "languages\Korean.isl"
Name: "croatian"; MessagesFile: "languages\Croatian.isl"
Name: "farsi"; MessagesFile: "languages\Farsi.isl"
Name: "dutch"; MessagesFile: "compiler:Languages\Dutch.isl"
Name: "hebrew"; MessagesFile: "compiler:Languages\Hebrew.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"
Name: "ukrainian"; MessagesFile: "compiler:Languages\Ukrainian.isl"
Name: "arabic"; MessagesFile: "languages\Arabic.isl"
Name: "thai"; MessagesFile: "languages\Thai.isl"
Name: "bengali"; MessagesFile: "languages\Bengali.islu"
Name: "hindi"; MessagesFile: "languages\Hindi.islu"
Name: "indonesian"; MessagesFile: "languages\Indonesian.isl"
Name: "urdu"; MessagesFile: "languages\Urdu.isl"
Name: "vietnamese"; MessagesFile: "languages\Vietnamese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\icons\concat.ico"; DestDir: "{app}"; DestName: "128 pixeles video.ico"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\128 pixeles video"; Filename: "{app}\128 pixeles video.exe"; IconFilename: "{app}\128 pixeles video.ico"
; The way in when a graphics driver draws the window as coloured squares:
; it turns off Settings > General > Draw on the GPU, which that window hides.
Name: "{autoprograms}\128 pixeles video (modo seguro)"; Filename: "{app}\128 pixeles video.exe"; Parameters: "--software"; IconFilename: "{app}\128 pixeles video.ico"
Name: "{autodesktop}\128 pixeles video"; Filename: "{app}\128 pixeles video.exe"; IconFilename: "{app}\128 pixeles video.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\128 pixeles video.exe"; Description: "{cm:LaunchProgram,128 pixeles video}"; Flags: nowait postinstall skipifsilent
