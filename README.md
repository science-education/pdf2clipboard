# PDF Page to Clipboard (Windows / macOS / Web)

<div align="center">
  <img src="docs/images/demo.webp" width="800" alt="動作デモンストレーション動画" />
  <p><em>▲ 60FPSの滑らかなサムネイル拡大・縮小と、ワンクリックでの高品質クリップボード転送デモ</em></p>
</div>

PDFのページをサムネイル一覧で表示し、選択したページの画像を高品質でクリップボードにコピーする究極の生産性ツールです。
Windows/macOS向けネイティブデスクトップアプリとして動作するほか、WebAssembly (Wasm) を通じてモダンなWebブラウザ上でも動作する完全クロスプラットフォーム構成となっています。

## 💡 直感的な使い方 (3ステップ)

```
 [ PDFファイルの読み込み ] ──(ドラッグ＆ドロップ または URL入力)──> [ 60FPSでプレビュー調整 ] ──(ワンクリック)──> [ 高品質で貼り付け! ]
```

<div align="center">
  <img src="docs/images/screenshot.png" width="800" alt="直感的なユーザーインターフェース" />
</div>

### 1. PDFを読み込む
- **ローカルファイル**: アプリケーション画面に PDF ファイルを直接**ドラッグ＆ドロップ**します。
- **オンラインファイル**: 画面上部の「URL:」欄に Web 上の PDF の URL を入力し、「開く」ボタンまたは Enter キーを押します。自動で CORS 制限や WAF をバイパスし、セキュアに非同期ダウンロードを行います。

### 2. プレビューと品質を調整する
- **プレビュー枠の変更**: 上部バーの「プレビュー:」スライダーを水平にドラッグすると、**60FPSでヌルヌルと滑らかにサムネイルサイズが変化**します。大量のページから目的の図表を一瞬で見つけ出せます。
- **解像度の選択**: 「DPI:」スライダーで、出力したい画質（72〜600 DPI、標準300 DPI）を自由に設定できます。

### 3. ワンクリックでクリップボードへ！
- コピーしたいページのサムネイルを**クリック**（またはキーボードの矢印キーで選択して Enter / Space）するだけ！
- 指定した高品質解像度の画像データが瞬時に生成され、システムのクリップボードに直接格納されます。そのまま Word や PowerPoint、Slack、メール等へ貼り付け（`Ctrl+V` / `Cmd+V`）できます。

---

## 🚀 主な機能

- **爆速・軽量なWASMレンダリング**: Web版ではブラウザネイティブのPDFエンジン（PDF.js）を統合。画像主体の重いPDFでも、100ページ超のドキュメントを瞬時にプレビュー可能です。
- **洗練されたUI/UXデザイン**: 各行が中央揃えされた精密なレイアウトと、マウスホバー時のダイナミックな青色ハイライト効果。
- **堅牢なCORSプロキシチェーン（Web版）**: Webブラウザのセキュリティ制限（CORS）を自動的に克服するため、複数の有力プロキシサーバー（`codetabs`, `corsproxy.io`, `allorigins`）を順番に試行する強力なフォールバック機能を搭載。官公庁や論文のPDFでも極めて高いダウンロード成功率を誇ります。
- **日本語フォントの完全バンドル**: Web版でも日本語やCJKテキストが「豆腐（□□□）」にならないよう、高品質ユニバーサルデザインフォント「Morisawa BIZ UD Gothic」を静的アセットとしてバイナリに完全統合。外部通信エラーや遅延なしに、瞬時に美しい日本語を表示します。
- **進捗の視覚化**: レンダリングやコピーの状況、URLダウンロード中のスピナー表示などをリアルタイムでステータスバーに表示し、操作の完了を迷わせません。

## 💻 開発・ビルド方法

### 必要環境
- Rust 1.75以上（[rustup](https://rustup.rs/) で導入）
- WebAssemblyビルド用: `wasm32-unknown-unknown` ターゲットおよび [Trunk](https://trunkrs.dev/) ビルドツール

### 1. ネイティブデスクトップ版（Windows / macOS）の実行
WindowsではWin32クリップボードAPI、macOSでは`arboard`経由のネイティブクリップボードを使用します。

```bash
cargo run --release
```
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
ローカル開発サーバーが起動し、ブラウザ（ `http://127.0.0.1:8080` ）上で即座に確認・テストが可能です。

## 🏛 ライセンス・リポジトリ

- **本体ソフトウェア**: MIT License
- **同梱フォント (BIZ UD Gothic)**: [SIL Open Font License 1.1 (OFL 1.1)](https://openfontlicense.org) (Copyright © 2022 The BIZ UDGothic Project Authors)
- **リポジトリ**: [https://github.com/science-education/pdf2clipboard](https://github.com/science-education/pdf2clipboard)
- **公開Web版**: [https://pdf2clipboard.pages.dev](https://pdf2clipboard.pages.dev)
