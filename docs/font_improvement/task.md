# 表示されない文字の根本修正タスク

- `[x]` 修正A: render_cid_direct にグリフ単位フォールバックを追加
- `[x]` 修正B: render_unicode_text のフォールバックを段階的（排他→順次）に変更
- `[x]` 修正C: is_cjk_or_kana を拡張（一般記号・句読点カバー）
- `[x]` ビルド検証（dev + release 成功、エラー・警告なし）
