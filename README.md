# PDFPageCopier

PDFのページをサムネイル一覧で表示し、選択したページの画像をクリップボードにコピーするWindows向けデスクトップアプリです。

## 機能

- PDFをドラッグ＆ドロップまたはファイルダイアログで開く
- 全ページをサムネイル形式で一覧表示
- ページをクリックするとクリップボードに高解像度画像（BMP形式）をコピー
- DPIとサムネイルサイズをスライダーで調整
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

## フォントレンダリング

PDFに埋め込まれたフォントを最優先で使用します。

対応している埋め込みフォント形式:

| 形式 | ストリーム | 対応 |
|------|-----------|------|
| TrueType サブセット | FontFile2 | ✅ |
| CFF / OpenType-CFF | FontFile3 | ✅ |
| CIDFontType0 (CFF, Identity-H/V) | FontFile3 | ✅ |
| CIDFontType2 (TrueType, Identity-H/V) | FontFile2 | ✅ |
| Type 1 | FontFile | システムフォントで代替 |

埋め込みフォントに該当グリフが無い場合は、システムフォント（CJK優先順、Latin優先順）へ自動フォールバックします。縦書きPDFは `-V` / `Vert` CMAPで検出し対応します。

### 実装上の注意点

- **CFF サブセットの CID→GID マップ**: CIDFontType0 (CFF) サブセットフォントでは、グリフは GID 順に格納されるが CID は CFF の `charset` テーブルが定義します。PDF の CID をそのまま GID として使うと別の glyph が描画されるため、`charset` テーブルから CID→GID 対応表を構築して使用します。
- **OpenType ラッパーの maxp.numGlyphs**: 生 CFF データを OpenType ラッパーで包む際、`maxp.numGlyphs` は 0xFFFF に設定。256 グリフ上限だと大半の日本語グリフが範囲外となり ttf-parser が描画を拒否します。
- **`re` オペレータの巻数方向**: PDF 仕様 §8.5.2.1 通り、`x y w h re` は w, h の符号によって巻数方向が変わります。負の寸法を正規化せず、`m`/`l`/`l`/`l`/`h` で展開することで「外側 CCW + 内側 CW」の組み合わせを保ち、非ゼロ巻数則 (`f`) で穴を正しく抜けます（例: 「文部科学省」ロゴ文字の白抜き部分）。

## 依存クレート

| クレート | 用途 |
|---------|------|
| [eframe](https://github.com/emilk/egui) / [egui](https://github.com/emilk/egui) | GUIフレームワーク |
| pdf_oxide (ローカルフォーク) | PDFパース・レンダリング |
| [windows](https://github.com/microsoft/windows-rs) | クリップボード操作 |

## ライセンス

MIT
