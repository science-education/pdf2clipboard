# タスクリスト: 高解像度展開とクリップボード処理の高速化

- `[x]` 1. `pdf_oxide_local/src/extractors/images.rs` で `zune-jpeg` を CMYK デコードに適用
- `[x]` 2. `pdf_oxide_local/Cargo.toml` に `zune-core` の明示的な依存追加（オプション設定に必要）
- `[x]` 3. `src/main.rs` の `clipboard_set_windows` をバックグラウンドスレッド化 & `rayon` 並列化
- `[x]` 4. ビルドおよび高解像度 PDF での動作確認
