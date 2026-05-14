# タスクリスト: サムネイル作成時のJPEG高速デコード導入（zune-jpeg版）

- `[x]` 1. `pdf_oxide_local/Cargo.toml` への `zune-jpeg` 依存関係の追加
- `[x]` 2. `pdf_oxide_local/src/extractors/images.rs` 内の `ImageData::Jpeg` 展開処理を `zune-jpeg` 優先の高速パスへ改修
- `[x]` 3. ビルドおよび動作検証（ネイティブ環境・WASM環境の互換性チェック）
