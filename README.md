# PDFPageCopier

PDFのページをサムネイル一覧で表示し、選択したページの画像をクリップボードにコピーするWindows向けデスクトップアプリです。

## 機能

- PDFをドラッグ＆ドロップまたはファイルダイアログで開く
- 全ページをサムネイル形式で一覧表示
- ページをクリックするとクリップボードに高解像度画像（BMP形式）をコピー
- DPIとサムネイルサイズをスライダーで調整
- CJK（日中韓）フォントおよびラテン文字フォントをアプリ内で選択・保存
- マルチスレッドでページを並列レンダリング

## ビルド方法

### 必要環境

- Rust 1.75以上（[rustup](https://rustup.rs/) で導入）
- Windows 10/11（Win32クリップボードAPIを使用）

### ビルド

```
cargo build --release
```

実行ファイルは `target/release/PDFPageCopier.exe` に生成されます。

## 設定

フォント設定は `%APPDATA%\PDFPageCopier\settings.json` に保存されます。

```json
{"cjk_font":"Meiryo","latin_font":"Arial"}
```

- `cjk_font`: 日本語・中国語・韓国語の文字に使用するフォント（空欄で自動選択）
- `latin_font`: 英数字・ラテン文字に使用するフォント（空欄でPDF埋め込みフォントのみ）

## フォントレンダリングの仕組み

PDFに埋め込まれたフォントを最優先で使用します。埋め込みフォントが参照できない文字については、アプリで選択したCJK・ラテンフォントで補完します。

対応している埋め込みフォント形式:

| 形式 | ストリーム | 対応 |
|------|-----------|------|
| TrueType サブセット | FontFile2 | ✅ |
| OpenType/CFF | FontFile3 | ✅ |
| CID (Identity-H/V) | FontFile2 | ✅ |
| Type 1 | FontFile | システムフォントで代替 |

縦書き（縦組み）PDFは `-V` または `Vert` CMAPを持つフォントで自動検出し対応します。

## 依存クレート

| クレート | 用途 |
|---------|------|
| [eframe](https://github.com/emilk/egui) / [egui](https://github.com/emilk/egui) | GUIフレームワーク |
| pdf_oxide (ローカルフォーク) | PDFパース・レンダリング |
| [windows](https://github.com/microsoft/windows-rs) | クリップボード操作 |

## ライセンス

MIT
