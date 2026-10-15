#!/usr/bin/env python3
"""生成 SnapStudio 跨平台应用图标集（master 1024 + PNGs + ico + icns）。"""
import io
import struct
import zlib
from PIL import Image, ImageDraw

OUT = "src-tauri/icons"
SIZE = 1024
MARGIN = 56
RADIUS = 220

C0 = (124, 92, 255)   # 紫
C1 = (33, 212, 253)   # 青


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def make_master():
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 圆角方块遮罩
    mask = Image.new("L", (SIZE, SIZE), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [MARGIN, MARGIN, SIZE - MARGIN, SIZE - MARGIN], radius=RADIUS, fill=255
    )

    # 对角线渐变填充
    grad = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    gp = grad.load()
    for y in range(SIZE):
        for x in range(0, SIZE, 4):  # 步长加速，再补插值
            t = (x + y) / (2 * SIZE)
            col = lerp(C0, C1, min(1.0, max(0.0, t)))
            for dx in range(4):
                if x + dx < SIZE:
                    gp[x + dx, y] = (col[0], col[1], col[2], 255)
    grad = Image.composite(grad, Image.new("RGBA", (SIZE, SIZE)), mask)
    img = Image.alpha_composite(img, grad)

    # 顶部高光
    hi = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    ImageDraw.Draw(hi).rounded_rectangle(
        [MARGIN, MARGIN, SIZE - MARGIN, MARGIN + (SIZE - 2 * MARGIN) // 2],
        radius=RADIUS, fill=(255, 255, 255, 38),
    )
    hi = Image.composite(hi, Image.new("RGBA", (SIZE, SIZE)), mask)
    img = Image.alpha_composite(img, hi)

    # 白色选框四角（截图符号）
    d = ImageDraw.Draw(img)
    arm = 150          # 角臂长
    off = 252          # 四角相对方块内边距
    lw = 36            # 线宽
    white = (255, 255, 255, 240)
    for (cx, cy) in [
        (off, off), (SIZE - off, off), (off, SIZE - off), (SIZE - off, SIZE - off)
    ]:
        horizontal = cx < SIZE / 2
        if horizontal and cy < SIZE / 2:        # 左上
            d.line([(cx, cy + lw // 2), (cx + arm, cy + lw // 2)], fill=white, width=lw)
            d.line([(cx + lw // 2, cy), (cx + lw // 2, cy + arm)], fill=white, width=lw)
        elif (not horizontal) and cy < SIZE / 2:  # 右上
            d.line([(cx, cy + lw // 2), (cx - arm, cy + lw // 2)], fill=white, width=lw)
            d.line([(cx - lw // 2, cy), (cx - lw // 2, cy + arm)], fill=white, width=lw)
        elif horizontal and cy >= SIZE / 2:       # 左下
            d.line([(cx, cy - lw // 2), (cx + arm, cy - lw // 2)], fill=white, width=lw)
            d.line([(cx + lw // 2, cy), (cx + lw // 2, cy - arm)], fill=white, width=lw)
        else:                                     # 右下
            d.line([(cx, cy - lw // 2), (cx - arm, cy - lw // 2)], fill=white, width=lw)
            d.line([(cx - lw // 2, cy), (cx - lw // 2, cy - arm)], fill=white, width=lw)

    # 中心十字准星
    cc = SIZE // 2
    d.line([(cc - 46, cc), (cc + 46, cc)], fill=white, width=14)
    d.line([(cc, cc - 46), (cc, cc + 46)], fill=white, width=14)
    d.ellipse([cc - 26, cc - 26, cc + 26, cc + 26], outline=white, width=12)
    return img


def png_bytes(im):
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    return buf.getvalue()


def build_icns(master):
    """手写现代 ICNS：每个表示均为 PNG（ic07/ic08/ic09/ic10）。"""
    reps = {
        b"ic07": 128,
        b"ic08": 256,
        b"ic09": 512,
        b"ic10": 1024,
        b"ic11": 32,    # 32@2x
        b"ic12": 64,    # 64@2x
        b"ic13": 256,   # 256@2x
        b"ic14": 512,   # 512@2x
    }
    entries = b""
    for code, s in reps.items():
        png = png_bytes(master.resize((s, s), Image.LANCZOS))
        entries += code + struct.pack(">I", len(png) + 8) + png
    return b"icns" + struct.pack(">I", len(entries) + 8) + entries


def main():
    import os
    os.makedirs(OUT, exist_ok=True)
    master = make_master()
    master.save(f"{OUT}/icon-master.png")

    # 标准 PNG 尺寸
    master.resize((32, 32), Image.LANCZOS).save(f"{OUT}/32x32.png")
    master.resize((128, 128), Image.LANCZOS).save(f"{OUT}/128x128.png")
    master.resize((256, 256), Image.LANCZOS).save(f"{OUT}/128x128@2x.png")

    # Windows .ico（多分辨率）
    ico_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    master.save(
        f"{OUT}/icon.ico",
        sizes=ico_sizes,
        format="ICO",
    )
    print("icon.ico written with", ico_sizes)

    # macOS .icns（手写 PNG 表示）
    with open(f"{OUT}/icon.icns", "wb") as f:
        f.write(build_icns(master))
    print("icon.icns written")

    print("master size:", master.size)
    print("icons generated at", OUT)


if __name__ == "__main__":
    main()
