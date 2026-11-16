"""从 assets/akribis-logo.png 生成 Windows 应用图标 assets/app.ico。

logo 是横向的「图形 + Akribis」字标（248x59，RGBA）。字标做成图标会看不清，
所以这里只取左侧图形部分（alpha 分析显示图形占 x=0..58，文字从 x=76 开始），
居中放到白色方形画布上，再输出多尺寸 ICO。

用法（在 app 目录下）：python scripts/make_icon.py
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets" / "akribis-logo.png"
TARGET = ROOT / "assets" / "app.ico"

GLYPH_RIGHT = 59  # 图形右边界（不含），由 alpha 列分析得出
CANVAS = 256
GLYPH_SIZE = 232  # 左右各留 12px 白边
ICON_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def main() -> None:
    logo = Image.open(SOURCE).convert("RGBA")
    glyph = logo.crop((0, 0, GLYPH_RIGHT, logo.height))

    side = max(glyph.size)
    scaled = glyph.resize((GLYPH_SIZE, GLYPH_SIZE), Image.LANCZOS)

    canvas = Image.new("RGBA", (CANVAS, CANVAS), (255, 255, 255, 255))
    offset = ((CANVAS - scaled.width) // 2, (CANVAS - scaled.height) // 2)
    canvas.paste(scaled, offset, scaled)

    TARGET.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(TARGET, format="ICO", sizes=ICON_SIZES)
    print(f"wrote {TARGET} ({side}x{side} glyph, sizes: {len(ICON_SIZES)})")


if __name__ == "__main__":
    main()
