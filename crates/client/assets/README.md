# Client assets

Sprites are painted in code (`src/textures.rs`); the only file here is the UI font.

`NotoSansSC-Subset.ttf` is [Noto Sans SC](https://fonts.google.com/noto/specimen/Noto+Sans+SC)
(SIL Open Font License 1.1, see `NotoSansSC-OFL.txt`), instanced at weight 500 and subset to
GB2312 + ASCII + every non-ASCII character in `crates/**/*.rs`, so browsers get Chinese text
without system fonts. After adding new Chinese UI strings outside GB2312, regenerate it with
fonttools from the upstream `NotoSansSC[wght].ttf`:

```bash
fonttools varLib.instancer 'NotoSansSC[wght].ttf' wght=500 -o inst.ttf
pyftsubset inst.ttf --text-file=chars.txt --layout-features='*' --no-hinting \
  --output-file=NotoSansSC-Subset.ttf
```

where `chars.txt` holds ASCII, all GB2312 characters and the non-ASCII characters of the sources.
