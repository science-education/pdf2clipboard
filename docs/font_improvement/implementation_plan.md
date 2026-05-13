# 表示されない文字の根本修正

## 根本原因の分析

コードの静的分析と MuPDF のフォント処理アーキテクチャとの比較から、**3つの構造的欠陥**を特定した。

### 欠陥1: `render_cid_direct` にグリフ単位フォールバックがない（致命的）

`render_cid_direct` は埋め込みフォント（byte-indexed TrueType, CFF, CIDFont）のレンダリングで最も頻繁に使われるパスだが、**GID=0（notdef）のグリフが見つからない場合、何も描画しない**。

```rust
// 現状: GID=0 のとき何もしない（静かにスキップ）
if gid != 0 || char_at_pos.is_whitespace() {
    if !char_at_pos.is_whitespace() {
        // 描画
    }
}
// ← GID=0 かつ空白でない場合の else ブロックがない
```

**MuPDFの対応**: `ft_char_index` で GID=0 が返った場合、グリフ名マッチング→Unicode再マッピング→代替フォント検索の3段階フォールバックを実行する。

### 欠陥2: `render_unicode_text` のフォールバックがCJK/非CJKの排他分岐（重大）

```rust
if !is_cjk_or_kana(char_at_pos) {
    // Latinフォールバックのみ
} else if is_cjk_or_kana(char_at_pos) {
    // CJKフォールバックのみ  
}
```

問題:
- Latinフォールバックでグリフが見つからなくても、CJKフォールバックを試みない
- CJKフォールバックでグリフが見つからなくても、Latinフォールバックを試みない
- 全角句読点（U+3000-U+303F）はCJK範囲だが、CJKフォントにない場合Latinは試みない

### 欠陥3: `is_cjk_or_kana` の範囲に一般的な日本語記号が欠落

以下の範囲がカバーされていない:
- U+2000-U+206F: 一般句読点（…、—、–、''、""等）
- U+2100-U+214F: 文字様記号（℃、№、℡等）
- U+2190-U+21FF: 矢印
- U+2200-U+22FF: 数学記号
- U+2500-U+257F: 罫線文字
- U+25A0-U+25FF: 幾何学図形（■、●、▲等）
- U+2600-U+26FF: 雑多な記号（☆、♪等）
- U+3000-U+303F: CJK記号・句読点（。、「」、『』等）← **これが入っていない!**

> [!IMPORTANT]
> `0x2E80..=0x9FFF` は U+3000-U+303F を含むが、U+2000-U+2E7F の一般記号は含まない

## 修正方針

### 修正A: `render_cid_direct` にグリフ単位フォールバックを追加
- GID=0 かつ Unicode文字が判明している場合、Latin → CJK → 汎用フォントの順でフォールバック

### 修正B: `render_unicode_text` のフォールバックを段階的に変更
- Latin → CJK → 汎用 の順に全文字に対してフォールバック（排他分岐を撤廃）

### 修正C: `is_cjk_or_kana` を拡張
- 一般句読点、数学記号、罫線、幾何学図形等を追加して `needs_cjk_font` に改名

## 変更ファイル

#### [MODIFY] [text_rasterizer.rs](file:///c:/Users/user/pdf_copier_rs/pdf_oxide_local/src/rendering/text_rasterizer.rs)

## 検証
- `cargo check` でビルド確認
