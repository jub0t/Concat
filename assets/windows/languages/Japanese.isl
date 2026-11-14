; *** Inno Setup version 6.5.0+ Japanese messages ***
;
; To download user-contributed translations of this file, go to:
;   https://jrsoftware.org/files/istrans/
;
; Note: When translating this text, do not add periods (.) to the end of
; messages that didn't have them already, because on those messages Inno
; Setup adds the periods automatically (appending a period would result in
; two periods being displayed).
;
[LangOptions]
; The following three entries are very important. Be sure to read and 
; understand the '[LangOptions] section' topic in the help file.
LanguageName=日本語
; If Language Name display incorrect, uncomment next line
; LanguageName=<65E5><672C><8A9E>
; About LanguageID, to reference link:
; https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-lcid/a9eac961-e77d-41a6-90a5-ce1a8b0cdb9c
LanguageID=$0411
; About CodePage, to reference link:
; https://docs.microsoft.com/en-us/windows/win32/intl/code-page-identifiers
LanguageCodePage=932
; If the language you are translating to requires special font faces or
; sizes, uncomment any of the following entries and change them accordingly.
;DialogFontName=
;DialogFontSize=9
;DialogFontBaseScaleWidth=7
;DialogFontBaseScaleHeight=15
;WelcomeFontName=Segoe UI
;WelcomeFontSize=14

[Messages]

; *** アプリケーションタイトル
SetupAppTitle=セットアップ
SetupWindowTitle=セットアップ - %1
UninstallAppTitle=アンインストール
UninstallAppFullTitle=%1 のアンインストール

; *** Misc. common
InformationTitle=情報
ConfirmTitle=確認
ErrorTitle=エラー

; *** SetupLdr messages
SetupLdrStartupMessage=これから %1 をインストールします。続行しますか？
LdrCannotCreateTemp=一時ファイルを作成できません。セットアップを中止しました
LdrCannotExecTemp=一時ディレクトリ内のファイルを実行できません。セットアップを中止しました
HelpTextNote=

; *** 起動時のエラーメッセージ
LastErrorMessage=%1。%n%nエラー %2: %3
SetupFileMissing=セットアップディレクトリにファイル %1 がありません。この問題を修正するか、プログラムの新しいコピーを入手してください。
SetupFileCorrupt=セットアップファイルが破損しています。プログラムの新しいコピーを入手してください。
SetupFileCorruptOrWrongVer=セットアップファイルが破損しているか、このセットアッププログラムのバージョンと互換性がありません。この問題を修正するか、プログラムの新しいコピーを入手してください。
InvalidParameter=無効なコマンドライン引数です：%n%n%1
SetupAlreadyRunning=セットアッププログラムは既に実行中です。
WindowsVersionNotSupported=このプログラムは、現在コンピュータで実行されている Windows のバージョンではサポートされていません。
WindowsServicePackRequired=このプログラムには %1 サービスパック %2 以降が必要です。
NotOnThisPlatform=このプログラムは %1 では実行できません。
OnlyOnThisPlatform=このプログラムは %1 でのみ実行できます。
OnlyOnTheseArchitectures=このプログラムは、次のプロセッサアーキテクチャ向けに設計された Windows にのみインストールできます：%n%n%1
WinVersionTooLowError=このプログラムには %1 バージョン %2 以降が必要です。
WinVersionTooHighError=このプログラムは %1 バージョン %2 以降にはインストールできません。
AdminPrivilegesRequired=このプログラムをインストールするには、管理者としてログオンする必要があります。
PowerUserPrivilegesRequired=このプログラムをインストールするには、管理者または Power Users グループの権限を持つユーザーとしてログオンする必要があります。
SetupAppRunningError=セットアッププログラムは %1 が現在実行中であることを検出しました。%n%n実行中のプログラムを閉じてから「OK」をクリックして続行するか、「キャンセル」をクリックして終了してください。
UninstallAppRunningError=アンインストールプログラムは %1 が現在実行中であることを検出しました。%n%n実行中のプログラムを閉じてから「OK」をクリックして続行するか、「キャンセル」をクリックして終了してください。

; *** 起動時の問題
PrivilegesRequiredOverrideTitle=セットアッププログラムのモードを選択
PrivilegesRequiredOverrideInstruction=インストールモードを選択してください
PrivilegesRequiredOverrideText1=%1 はすべてのユーザーにインストールするか (管理者権限が必要)、現在のユーザーのみにインストールできます。
PrivilegesRequiredOverrideText2=%1 は現在のユーザーのみにインストールするか、すべてのユーザーにインストールできます (管理者権限が必要)。
PrivilegesRequiredOverrideAllUsers=すべてのユーザーにインストール(&A)
PrivilegesRequiredOverrideAllUsersRecommended=すべてのユーザーにインストール(&A) (推奨)
PrivilegesRequiredOverrideCurrentUser=現在のユーザーのみにインストール(&M)
PrivilegesRequiredOverrideCurrentUserRecommended=現在のユーザーのみにインストール(&M) (推奨)

; *** その他のエラー
ErrorCreatingDir=セットアッププログラムはディレクトリ「%1」を作成できませんでした
ErrorTooManyFilesInDir=ディレクトリ「%1」内にファイルが多すぎるため、ファイルを作成できません

; *** セットアッププログラム共通メッセージ
ExitSetupTitle=セットアッププログラムを終了
ExitSetupMessage=セットアップは完了していません。今終了すると、このプログラムはインストールされません。%n%n後でセットアッププログラムを再度実行してインストールを完了できます。%n%nセットアッププログラムを終了しますか？
AboutSetupMenuItem=セットアッププログラムについて(&A)...
AboutSetupTitle=セットアッププログラムについて
AboutSetupMessage=%1 バージョン %2%n%3%n%n%1 ホームページ：%n%4
AboutSetupNote=
TranslatorNote=日本語翻訳

; *** ボタン
ButtonBack=< 戻る(&B)
ButtonNext=次へ(&N) >
ButtonInstall=インストール(&I)
ButtonOK=OK
ButtonCancel=キャンセル
ButtonYes=はい(&Y)
ButtonYesToAll=すべてはい(&A)
ButtonNo=いいえ(&N)
ButtonNoToAll=すべていいえ(&O)
ButtonFinish=完了(&F)
ButtonBrowse=参照(&B)...
ButtonWizardBrowse=参照(&R)...
ButtonNewFolder=新しいフォルダー(&M)

; *** 「言語の選択」ダイアログメッセージ
SelectLanguageTitle=セットアップ言語の選択
SelectLanguageLabel=セットアップ時に使用する言語を選択してください。

; *** 共通ウィザードテキスト
ClickNext=「次へ」をクリックして続行するか、「キャンセル」をクリックしてセットアッププログラムを終了してください。
BeveledLabel=
BrowseDialogTitle=フォルダーの参照
BrowseDialogLabel=下の一覧からフォルダーを選択し、「OK」をクリックしてください。
NewFolderName=新しいフォルダー

; *** 「ようこそ」ウィザードページ
WelcomeLabel1=[name] セットアップウィザードへようこそ
WelcomeLabel2=これから [name/ver] をコンピュータにインストールします。%n%nインストールを続行する前に、他のすべてのアプリケーションを閉じることをお勧めします。

; *** 「パスワード」ウィザードページ
WizardPassword=パスワード
PasswordLabel1=このセットアッププログラムはパスワードで保護されています。
PasswordLabel3=パスワードを入力して「次へ」をクリックし、続行してください。パスワードは大文字と小文字を区別します。
PasswordEditLabel=パスワード(&P)：
IncorrectPassword=入力したパスワードが正しくありません。もう一度入力してください。

; *** 「ライセンス契約」ウィザードページ
WizardLicense=ライセンス契約
LicenseLabel=インストールを続行する前に、次の重要事項をお読みください。
LicenseLabel3=次のライセンス契約をよくお読みください。インストールを続行するには、この契約条項に同意する必要があります。
LicenseAccepted=この契約に同意します(&A)
LicenseNotAccepted=この契約に同意しません(&D)

; *** 「情報」ウィザードページ
WizardInfoBefore=情報
InfoBeforeLabel=インストールを続行する前に、次の重要事項をお読みください。
InfoBeforeClickLabel=インストールを続行する準備ができたら、「次へ」をクリックしてください。
WizardInfoAfter=情報
InfoAfterLabel=インストールを続行する前に、次の重要事項をお読みください。
InfoAfterClickLabel=インストールを続行する準備ができたら、「次へ」をクリックしてください。

; *** 「ユーザー情報」ウィザードページ
WizardUserInfo=ユーザー情報
UserInfoDesc=情報を入力してください。
UserInfoName=ユーザー名(&U)：
UserInfoOrg=組織(&O)：
UserInfoSerial=シリアル番号(&S)：
UserInfoNameRequired=ユーザー名を入力する必要があります。

; *** 「インストール先の選択」ウィザードページ
WizardSelectDir=インストール先の選択
SelectDirDesc=[name] をどこにインストールしますか？
SelectDirLabel3=セットアッププログラムは [name] を次のフォルダーにインストールします。
SelectDirBrowseLabel=「次へ」をクリックして続行してください。別のフォルダーを選択する場合は、「参照」をクリックしてください。
DiskSpaceGBLabel=少なくとも [gb] GB の空きディスク容量が必要です。
DiskSpaceMBLabel=少なくとも [mb] MB の空きディスク容量が必要です。
CannotInstallToNetworkDrive=セットアッププログラムはネットワークドライブにインストールできません。
CannotInstallToUNCPath=セットアッププログラムは UNC パスにインストールできません。
InvalidPath=ドライブレターを含む完全なパスを入力する必要があります。例：%n%nC:\APP%n%nまたは UNC パス：%n%n\\server\share
InvalidDrive=選択したドライブまたは UNC 共有が存在しないか、アクセスできません。別の場所を選択してください。
DiskSpaceWarningTitle=ディスク容量不足
DiskSpaceWarning=セットアッププログラムにはインストールに少なくとも %1 KB の空き容量が必要ですが、選択したドライブには %2 KB の空き容量しかありません。%n%n続行しますか？
DirNameTooLong=フォルダー名またはパスが長すぎます。
InvalidDirName=フォルダー名が無効です。
BadDirName32=フォルダー名には次の文字を含めることはできません：%n%n%1
DirExistsTitle=フォルダーが既に存在します
DirExists=フォルダー：%n%n%1%n%nは既に存在します。このフォルダーにインストールしますか？
DirDoesntExistTitle=フォルダーが存在しません
DirDoesntExist=フォルダー：%n%n%1%n%nは存在しません。このフォルダーを作成しますか？

; *** 「コンポーネントの選択」ウィザードページ
WizardSelectComponents=コンポーネントの選択
SelectComponentsDesc=どのプログラムコンポーネントをインストールしますか？
SelectComponentsLabel2=インストールするコンポーネントを選択し、インストールしないコンポーネントのチェックを外します。その後、「次へ」をクリックして続行してください。
FullInstallation=完全インストール
; if possible don't translate 'Compact' as 'Minimal' (I mean 'Minimal' in your language)
CompactInstallation=コンパクトインストール
CustomInstallation=カスタムインストール
NoUninstallWarningTitle=コンポーネントが既に存在します
NoUninstallWarning=セットアッププログラムは、次のコンポーネントが既にコンピュータにインストールされていることを検出しました：%n%n%1%n%nこれらのコンポーネントのチェックを外してもアンインストールされません。%n%n続行しますか？
ComponentSize1=%1 KB
ComponentSize2=%1 MB
ComponentsDiskSpaceGBLabel=現在選択されているコンポーネントには少なくとも [gb] GB のディスク容量が必要です。
ComponentsDiskSpaceMBLabel=現在選択されているコンポーネントには少なくとも [mb] MB のディスク容量が必要です。

; *** 「追加タスクの選択」ウィザードページ
WizardSelectTasks=追加タスクの選択
SelectTasksDesc=セットアッププログラムで実行する追加タスクを選択してください。
SelectTasksLabel2=[name] のインストール時にセットアッププログラムで実行する追加タスクを選択し、「次へ」をクリックしてください。

; *** 「スタートメニューフォルダーの選択」ウィザードページ
WizardSelectProgramGroup=スタートメニューフォルダーの選択
SelectStartMenuFolderDesc=プログラムのショートカットをどこに作成しますか？
SelectStartMenuFolderLabel3=セットアッププログラムは次の「スタート」メニューフォルダーにプログラムのショートカットを作成します。
SelectStartMenuFolderBrowseLabel=「次へ」をクリックして続行してください。別のフォルダーを選択する場合は、「参照」をクリックしてください。
MustEnterGroupName=フォルダー名を入力する必要があります。
GroupNameTooLong=フォルダー名またはパスが長すぎます。
InvalidGroupName=フォルダー名が無効です。
BadGroupName=フォルダー名には次の文字を含めることはできません：%n%n%1
NoProgramGroupCheck2=スタートメニューフォルダーを作成しない(&D)

; *** 「インストールの準備」ウィザードページ
WizardReady=インストールの準備
ReadyLabel1=セットアッププログラムの準備ができました。これから [name] をコンピュータにインストールします。
ReadyLabel2a=「インストール」をクリックしてこのセットアッププログラムを続行してください。設定を再検討または変更する場合は、「戻る」をクリックしてください。
ReadyLabel2b=「インストール」をクリックしてこのセットアッププログラムを続行してください。
ReadyMemoUserInfo=ユーザー情報：
ReadyMemoDir=インストール先：
ReadyMemoType=インストールの種類：
ReadyMemoComponents=選択したコンポーネント：
ReadyMemoGroup=スタートメニューフォルダー：
ReadyMemoTasks=追加タスク：

; *** TExtractionWizardPage ウィザードページと ExtractArchive
ExtractingLabel=ファイルを展開しています...
ButtonStopExtraction=展開を停止(&S)
StopExtraction=展開を停止してもよろしいですか？
ErrorExtractionAborted=展開が中止されました
ErrorExtractionFailed=展開に失敗しました：%1

; *** 圧縮ファイル展開失敗の詳細
ArchiveIncorrectPassword=圧縮ファイルのパスワードが正しくありません
ArchiveIsCorrupted=圧縮ファイルが破損しています
ArchiveUnsupportedFormat=サポートされていない圧縮ファイル形式です

; *** TDownloadWizardPage ウィザードページと DownloadTemporaryFile
DownloadingLabel2=ファイルをダウンロードしています...
ButtonStopDownload=ダウンロードを停止(&S)
StopDownload=ダウンロードを停止してもよろしいですか？
ErrorDownloadAborted=ダウンロードが中止されました
ErrorDownloadFailed=ダウンロードに失敗しました：%1 %2
ErrorDownloadSizeFailed=ダウンロードサイズの取得に失敗しました：%1 %2
ErrorProgress=無効な進行状況：%1 / %2
ErrorFileSize=ファイルサイズが正しくありません：予期 %1、実際 %2

; *** 「インストールの準備中」ウィザードページ
WizardPreparing=インストールの準備中
PreparingDesc=セットアッププログラムは [name] をコンピュータにインストールする準備をしています。
PreviousInstallNotCompleted=以前のプログラムのインストールまたはアンインストールが完了していません。完了するにはコンピュータを再起動する必要があります。%n%nコンピュータを再起動した後、セットアッププログラムを再度実行して [name] のインストールを完了してください。
CannotContinue=セットアッププログラムを続行できません。「キャンセル」をクリックして終了してください。
ApplicationsFound=次のアプリケーションは、セットアッププログラムが更新するファイルを使用しています。セットアッププログラムがこれらのアプリケーションを自動的に閉じることを許可することをお勧めします。
ApplicationsFound2=次のアプリケーションは、セットアッププログラムが更新するファイルを使用しています。セットアッププログラムがこれらのアプリケーションを自動的に閉じることを許可することをお勧めします。インストール完了後、セットアッププログラムはこれらのアプリケーションを再起動しようとします。
CloseApplications=アプリケーションを自動的に閉じる(&A)
DontCloseApplications=アプリケーションを閉じない(&D)
ErrorCloseApplications=セットアッププログラムはすべてのアプリケーションを自動的に閉じることができませんでした。続行する前に、セットアッププログラムが更新するファイルを使用しているすべてのアプリケーションを閉じることをお勧めします。
PrepareToInstallNeedsRestart=セットアッププログラムはコンピュータを再起動する必要があります。コンピュータの再起動後、セットアッププログラムを再度実行して [name] のインストールを完了してください。%n%n今すぐ再起動しますか？

; *** 「インストール中」ウィザードページ
WizardInstalling=インストール中
InstallingLabel=セットアッププログラムは [name] をコンピュータにインストールしています。しばらくお待ちください。

; *** 「インストール完了」ウィザードページ
FinishedHeadingLabel=[name] のインストールが完了しました
FinishedLabelNoIcons=セットアッププログラムは [name] をコンピュータにインストールしました。
FinishedLabel=セットアッププログラムは [name] をコンピュータにインストールしました。インストールされたショートカットからこのアプリケーションを実行できます。
ClickFinish=「完了」をクリックしてセットアッププログラムを終了してください。
FinishedRestartLabel=[name] のインストールを完了するには、セットアッププログラムがコンピュータを再起動する必要があります。今すぐ再起動しますか？
FinishedRestartMessage=[name] のインストールを完了するには、セットアッププログラムがコンピュータを再起動する必要があります。%n%n今すぐ再起動しますか？
ShowReadmeCheck=はい、README ファイルを読みます
YesRadio=はい、今すぐコンピュータを再起動します(&Y)
NoRadio=いいえ、後でコンピュータを再起動します(&N)
; used for example as 'Run MyProg.exe'
RunEntryExec=%1 を実行
; used for example as 'View Readme.txt'
RunEntryShellExec=%1 を表示

; *** 「セットアッププログラムは次のディスクを要求しています」プロンプト
ChangeDiskTitle=セットアッププログラムは次のディスクを要求しています
SelectDiskLabel2=ディスク %1 を挿入して「OK」をクリックしてください。%n%nこのディスク内のファイルが次のフォルダー以外のフォルダーにある場合は、正しいパスを入力するか「参照」をクリックしてください。
PathLabel=パス(&P)：
FileNotInDir2=ファイル「%1」が「%2」に見つかりません。正しいディスクを挿入するか、別のフォルダーを選択してください。
SelectDirectoryLabel=次のディスクの場所を指定してください。

; *** インストール段階のメッセージ
SetupAborted=セットアッププログラムはインストールを完了しませんでした。%n%nこの問題を修正して、セットアッププログラムを再度実行してください。
AbortRetryIgnoreSelectAction=操作の選択
AbortRetryIgnoreRetry=再試行(&T)
AbortRetryIgnoreIgnore=エラーを無視して続行(&I)
AbortRetryIgnoreCancel=セットアッププログラムを閉じる
RetryCancelSelectAction=操作の選択
RetryCancelRetry=再試行(&T)
RetryCancelCancel=キャンセル(&C)

; *** インストール状態メッセージ
StatusClosingApplications=アプリケーションを閉じています...
StatusCreateDirs=ディレクトリを作成しています...
StatusExtractFiles=ファイルを展開しています...
StatusDownloadFiles=ファイルをダウンロードしています...
StatusCreateIcons=ショートカットを作成しています...
StatusCreateIniEntries=INI エントリを作成しています...
StatusCreateRegistryEntries=レジストリエントリを作成しています...
StatusRegisterFiles=ファイルを登録しています...
StatusSavingUninstall=アンインストール情報を保存しています...
StatusRunProgram=インストールを完了しています...
StatusRestartingApplications=アプリケーションを再起動しています...
StatusRollback=変更を元に戻しています...

; *** その他のエラー
ErrorInternal2=内部エラー：%1
ErrorFunctionFailedNoCode=%1 に失敗しました
ErrorFunctionFailed=%1 に失敗しました；エラーコード %2
ErrorFunctionFailedWithMessage=%1 に失敗しました；エラーコード %2.%n%3
ErrorExecutingProgram=ファイルを実行できません：%n%1

; *** レジストリエラー
ErrorRegOpenKey=レジストリキーを開くときにエラーが発生しました：%n%1\%2
ErrorRegCreateKey=レジストリキーを作成するときにエラーが発生しました：%n%1\%2
ErrorRegWriteKey=レジストリキーに書き込むときにエラーが発生しました：%n%1\%2

; *** INI エラー
ErrorIniEntry=ファイル「%1」に INI エントリを作成するときにエラーが発生しました。

; *** ファイルコピーエラー
FileAbortRetryIgnoreSkipNotRecommended=このファイルをスキップ(&S)（非推奨）
FileAbortRetryIgnoreIgnoreNotRecommended=エラーを無視して続行(&I)（非推奨）
SourceIsCorrupted=ソースファイルが破損しています
SourceDoesntExist=ソースファイル「%1」が存在しません
SourceVerificationFailed=ソースファイルの検証に失敗しました: %1
VerificationSignatureDoesntExist=署名ファイル「%1」が存在しません
VerificationSignatureInvalid=署名ファイル「%1」が無効です
VerificationKeyNotFound=署名ファイル「%1」は不明なキーを使用しています
VerificationFileNameIncorrect=ファイル名が正しくありません
VerificationFileTagIncorrect=ファイルタグが正しくありません
VerificationFileSizeIncorrect=ファイルサイズが正しくありません
VerificationFileHashIncorrect=ファイルハッシュ値が正しくありません
ExistingFileReadOnly2=既存のファイルを置き換えられません。読み取り専用です。
ExistingFileReadOnlyRetry=読み取り専用属性を解除して再試行(&R)
ExistingFileReadOnlyKeepExisting=既存のファイルを保持(&K)
ErrorReadingExistingDest=既存のファイルを読み取るときにエラーが発生しました：
FileExistsSelectAction=操作の選択
FileExists2=ファイルは既に存在します。
FileExistsOverwriteExisting=既存のファイルを上書き(&O)
FileExistsKeepExisting=既存のファイルを保持(&K)
FileExistsOverwriteOrKeepAll=すべての競合ファイルに対してこの操作を実行(&D)
ExistingFileNewerSelectAction=操作の選択
ExistingFileNewer2=既存のファイルは、セットアッププログラムがインストールしようとしているファイルよりも新しいです。
ExistingFileNewerOverwriteExisting=既存のファイルを上書き(&O)
ExistingFileNewerKeepExisting=既存のファイルを保持(&K)（推奨）
ExistingFileNewerOverwriteOrKeepAll=すべての競合ファイルに対してこの操作を実行(&D)
ErrorChangingAttr=次の既存ファイルの属性を変更するときにエラーが発生しました：
ErrorCreatingTemp=インストール先ディレクトリにファイルを作成するときにエラーが発生しました：
ErrorReadingSource=次のソースファイルを読み取るときにエラーが発生しました：
ErrorCopying=次のファイルをコピーするときにエラーが発生しました：
ErrorDownloading=ファイルのダウンロード中にエラーが発生しました：
ErrorExtracting=圧縮ファイルの展開中にエラーが発生しました：
ErrorReplacingExistingFile=既存のファイルを置き換えるときにエラーが発生しました：
ErrorRestartReplace=再起動後の置き換えに失敗しました：
ErrorRenamingTemp=インストール先ディレクトリ内のファイルの名前を変更するときにエラーが発生しました：
ErrorRegisterServer=DLL/OCX を登録できません：%1
ErrorRegSvr32Failed=RegSvr32 に失敗しました；終了コード %1
ErrorRegisterTypeLib=タイプライブラリを登録できません：%1

; *** アンインストール表示名のマーク
; used for example as 'My Program (32-bit)'
UninstallDisplayNameMark=%1 (%2)
; used for example as 'My Program (32-bit, All users)'
UninstallDisplayNameMarks=%1 (%2, %3)
UninstallDisplayNameMark32Bit=32 ビット
UninstallDisplayNameMark64Bit=64 ビット
UninstallDisplayNameMarkAllUsers=すべてのユーザー
UninstallDisplayNameMarkCurrentUser=現在のユーザー

; *** インストール後のエラー
ErrorOpeningReadme=README ファイルを開くときにエラーが発生しました。
ErrorRestartingComputer=セットアッププログラムはコンピュータを再起動できませんでした。手動で再起動してください。

; *** アンインストールメッセージ
UninstallNotFound=ファイル「%1」が存在しません。アンインストールできません。
UninstallOpenError=ファイル「%1」を開けません。アンインストールできません。
UninstallUnsupportedVer=このバージョンのアンインストールプログラムは、アンインストールログファイル「%1」の形式を認識できません。アンインストールできません
UninstallUnknownEntry=アンインストールログに不明なエントリ (%1) が見つかりました
ConfirmUninstall=%1 およびそのすべてのコンポーネントを完全に削除してもよろしいですか？
UninstallOnlyOnWin64=このプログラムは 64 ビット Windows でのみアンインストールできます。
OnlyAdminCanUninstall=管理者権限を持つユーザーのみがこのアンインストールを完了できます。
UninstallStatusLabel=コンピュータから %1 を削除しています。しばらくお待ちください。
UninstalledAll=%1 はコンピュータから正常に削除されました。
UninstalledMost=%1 のアンインストールが完了しました。%n%n一部の項目は削除できませんでしたが、手動で削除できます。
UninstalledAndNeedsRestart=%1 のアンインストールを完了するには、コンピュータを再起動する必要があります。%n%n今すぐコンピュータを再起動しますか？
UninstallDataCorrupted=ファイル「%1」が破損しています。アンインストールできません

; *** アンインストール状態メッセージ
ConfirmDeleteSharedFileTitle=共有ファイルを削除しますか？
ConfirmDeleteSharedFile2=次の共有ファイルは他のプログラムで使用されなくなったとシステムが判断しました。アンインストールプログラムでこれらの共有ファイルを削除しますか？%n%nこれらのファイルを削除しても、まだ使用しているプログラムがある場合は、それらのプログラムで問題が発生する可能性があります。不明な場合は「いいえ」を選択し、問題を避けるためにシステムにこれらのファイルを残してください。
SharedFileNameLabel=ファイル名：
SharedFileLocationLabel=場所：
WizardUninstalling=アンインストール状態
StatusUninstalling=%1 をアンインストールしています...

; *** Shutdown block reasons
ShutdownBlockReasonInstallingApp=%1 をインストールしています。
ShutdownBlockReasonUninstallingApp=%1 をアンインストールしています。

; The custom messages below aren't used by Setup itself, but if you make
; use of them in your scripts, you'll want to translate them.

[CustomMessages]

NameAndVersion=%1 バージョン %2
AdditionalIcons=追加のショートカット：
CreateDesktopIcon=デスクトップショートカットを作成(&D)
CreateQuickLaunchIcon=クイック起動ショートカットを作成(&Q)
ProgramOnTheWeb=%1 の Web サイト
UninstallProgram=%1 をアンインストール
LaunchProgram=%1 を実行
AssocFileExtension=%2 ファイル拡張子を %1 に関連付け(&A)
AssocingFileExtension=%2 ファイル拡張子を %1 に関連付けています...
AutoStartProgramGroupDescription=スタートアップ：
AutoStartProgram=%1 を自動起動
AddonHostProgramNotFound=選択したフォルダーに %1 が見つかりません。%n%n続行しますか？
