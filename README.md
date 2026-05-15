# PDF Page to Clipboard (Windows / macOS / Web)

PDFのページをサムネイル一覧で表示し、選択したページの画像を高品質でクリップボードにコピーするアプリケーションです。
Windows/macOS向けネイティブデスクトップアプリとして動作するほか、WebAssembly (Wasm) を通じてモダンなWebブラウザ上でも動作するクロスプラットフォーム構成となっています。

## 主な機能

- **爆速・軽量なWASMレンダリング**: Web版ではブラウザネイティブのPDFエンジン（PDF.js）を統合。画像主体の重いPDFでも、100ページ超のドキュメントを瞬時にプレビュー可能です。
- **洗練されたUI/UXデザイン**: 
    - 各行が中央揃えされた精密なメッセージ配置。
    - マウスホバー時のダイナミックなハイライト効果。
    - 選択（青）・完了（赤）を直感的に示す角丸ボーダーライン。
- **高品質クリップボード転送**: Windows/macOS/Webを問わず、指定した解像度（デフォルト300 DPI）で画像を生成し、そのままシステムのクリップボードへ格納。プレゼン資料やドキュメント作成への貼り付けに最適です。
- **柔軟なファイル読み込み**: PDFファイルをドラッグ＆ドロップするだけで即座に開始。日本語ファイル名にも完全対応しています。
- **進捗の視覚化**: レンダリングやコピーの状況をリアルタイムでステータスバーに表示。操作の完了を迷わせません。
- **堅牢なフォント描画（Desktop版）**: 縦書きPDFや埋め込みCFF/TrueTypeフォント、CJKフォールバック描画に対応した独自のレンダリングエンジンを搭載。

## 開発・ビルド方法

### 必要環境
- Rust 1.75以上（[rustup](https://rustup.rs/) で導入）
- WebAssemblyビルド用: `wasm32-unknown-unknown` ターゲットおよび [Trunk](https://trunkrs.dev/) ビルドツール

### 1. ネイティブデスクトップ版（Windows / macOS）の実行
WindowsではWin32クリップボードAPI、macOSでは`arboard`経由のネイティブクリップボードを使用します。

```bash
cargo run --release
```

ビルドされた実行ファイルは以下に出力されます。

- Windows: `target/release/PDFPageToClipboard.exe`
- macOS: `target/release/PDFPageToClipboard`

### 2. Webアプリ版（WebAssembly）の実行
PDF.jsによるハードウェア加速レンダリングとWeb標準のClipboard APIを利用します。

```bash
# Trunkが未インストールの場合は事前に導入してください
# cargo install trunk
# rustup target add wasm32-unknown-unknown

trunk serve
```
ローカル開発サーバーが起動し、ブラウザ（デフォルトでは `http://127.0.0.1:8080`）上で即座に確認・テストが可能です。

## 技術スタック・設計のポイント

- **Frontend**: [egui](https://github.com/emilk/egui) を採用し、イミディエイトモードGUIによる極めて高い応答性を実現。
- **Wasm Interop**: `wasm-bindgen` を介してJavaScript (PDF.js) と密に連携。重い解析と描画をブラウザ側にオフロードしつつ、Rust側で状態管理を行うハイブリッド構成。
- **Platform Layer**: `src/platform.rs` にWindows/macOS/Webのクリップボード処理を分離。各環境固有のAPI差分をアプリ本体から切り離しています。
- **Desktop Engine**: ローカルフォークされた `pdf_oxide` を使用。16MBの拡張スタックスレッドにより、複雑なPDFの解析時におけるオーバーフローを防止。
- **安定した非同期処理**: PDFロード、サムネイルレンダリング、コピー完了通知に世代管理を導入し、古い非同期結果が現在のPDF表示を上書きしないようにしています。
- **マルチ言語対応**: システムロケールを自動判別し、日本語/英語を切り替えて表示。

## ライセンス

MIT License

## リポジトリ
https://github.com/science-education/pdf2clipboard
