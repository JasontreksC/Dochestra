"""Generate the documents used by scripts/live-nodes.sh (committed output, <1 MB each).

    uv run --project extractor python fixtures/live/make_fixtures.py

Outputs (next to this file):
  vision_{chart,table,text,photo,small}.pdf  one image per PDF for the analyze node (T-313)
  gallery_en.pdf   English sensor atlas with 28 figures (more than the 20-image limit of summary)
  handbook_en.pdf  English technical handbook, 20+ pages: tables, code listings, figures
  report_en.pdf  English annual report, 4 pages, 2 tables, 2 figures
  report_ko.pdf  Korean report, 3 pages, 1 table, 1 figure (NanumGothic from engine/assets/fonts)
The facts in the text are what fixtures/live/questions.json asks about.
"""

import io
from pathlib import Path

from PIL import Image, ImageDraw
from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.utils import ImageReader
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import (
    Image as RLImage,
)
from reportlab.platypus import (
    PageBreak,
    Paragraph,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)

HERE = Path(__file__).resolve().parent
FONTS = HERE.parent.parent / "engine" / "assets" / "fonts"
PALETTE = ["#3E63DD", "#E5484D", "#30A46C", "#F5A524", "#8E4EC6"]


def bar_chart(values: list[int], labels: list[str], title: str) -> io.BytesIO:
    img = Image.new("RGB", (640, 380), "white")
    d = ImageDraw.Draw(img)
    d.text((20, 10), title, fill="black")
    d.line([50, 340, 620, 340], fill="black", width=2)
    d.line([50, 40, 50, 340], fill="black", width=2)
    top = max(values)
    for i, (v, lab) in enumerate(zip(values, labels, strict=True)):
        x = 80 + i * 130
        h = int(270 * v / top)
        d.rectangle([x, 340 - h, x + 80, 339], fill=PALETTE[i % len(PALETTE)])
        d.text((x, 345), lab, fill="black")
        d.text((x + 20, 325 - h), str(v), fill="black")
    return _jpeg(img)


def line_chart(values: list[int], title: str) -> io.BytesIO:
    img = Image.new("RGB", (640, 380), "white")
    d = ImageDraw.Draw(img)
    d.text((20, 10), title, fill="black")
    d.line([50, 340, 620, 340], fill="black", width=2)
    d.line([50, 40, 50, 340], fill="black", width=2)
    top = max(values)
    pts = [(80 + i * 110, 340 - int(270 * v / top)) for i, v in enumerate(values)]
    d.line(pts, fill="#E5484D", width=4)
    for (x, y), v in zip(pts, values, strict=True):
        d.ellipse([x - 6, y - 6, x + 6, y + 6], fill="#3E63DD")
        d.text((x - 10, y - 20), str(v), fill="black")
    return _jpeg(img)


def _jpeg(img: Image.Image) -> io.BytesIO:
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    buf.seek(0)
    return buf


def grid(data: list[list[str]], font: str, widths: list[float]) -> Table:
    t = Table(data, colWidths=widths)
    t.setStyle(
        TableStyle(
            [
                ("GRID", (0, 0), (-1, -1), 0.8, colors.black),
                ("BACKGROUND", (0, 0), (-1, 0), colors.lightgrey),
                ("FONTNAME", (0, 0), (-1, -1), font),
                ("ALIGN", (1, 1), (-1, -1), "RIGHT"),
            ]
        )
    )
    return t


def english(out: Path) -> None:
    ss = getSampleStyleSheet()
    body = ss["BodyText"]
    doc = SimpleDocTemplate(str(out), pagesize=A4, title="Aurora Robotics Annual Report 2025")
    story = [
        Paragraph("Aurora Robotics Annual Report 2025", ss["Title"]),
        Paragraph("1. Executive Summary", ss["Heading2"]),
        Paragraph(
            "Aurora Robotics, founded in 2012 in Busan, South Korea, designs warehouse "
            "automation robots. In 2025 the company reached a total revenue of 482 million "
            "US dollars, an increase of 18 percent over 2024. Chief Executive Officer "
            "Min-jun Park credits the growth to strong demand for autonomous picking "
            "systems and to the expansion of the service business.",
            body,
        ),
        Paragraph(
            "Operating margin improved to 14 percent as the company finished the migration "
            "of its assembly lines to a new plant in Gimhae. Headcount grew from 1,120 to "
            "1,340 employees, mostly in engineering and customer support.",
            body,
        ),
        Spacer(1, 6 * mm),
        Paragraph("2. Revenue by Region", ss["Heading2"]),
        Paragraph(
            "Asia-Pacific was the fastest growing region with growth of 31 percent, "
            "followed by North America at 17 percent. Europe grew moderately while the "
            "Middle East remained the smallest market.",
            body,
        ),
        grid(
            [
                ["Region", "2024 (USD M)", "2025 (USD M)", "Growth"],
                ["Asia-Pacific", "152", "199", "31%"],
                ["North America", "121", "142", "17%"],
                ["Europe", "105", "114", "9%"],
                ["Middle East", "30", "27", "-10%"],
            ],
            "Helvetica",
            [38 * mm, 34 * mm, 34 * mm, 28 * mm],
        ),
        Spacer(1, 6 * mm),
        RLImage(
            bar_chart([199, 142, 114, 27], ["APAC", "N. America", "Europe", "M. East"], "Revenue 2025 (USD M)"),
            width=120 * mm,
            height=71 * mm,
        ),
        Paragraph("Figure 1. Revenue by region in 2025.", body),
        PageBreak(),
        Paragraph("3. Product Lines", ss["Heading2"]),
        Paragraph(
            "The Atlas picking robot remained the best selling product with 4,200 units "
            "shipped. The Nova conveyor shuttle followed with 2,950 units, while the new "
            "Orbit inspection drone shipped its first 610 units in the fourth quarter.",
            body,
        ),
        grid(
            [
                ["Product", "Units 2024", "Units 2025"],
                ["Atlas picking robot", "3,300", "4,200"],
                ["Nova conveyor shuttle", "2,700", "2,950"],
                ["Orbit inspection drone", "0", "610"],
            ],
            "Helvetica",
            [60 * mm, 36 * mm, 36 * mm],
        ),
        Spacer(1, 6 * mm),
        RLImage(
            line_chart([310, 340, 395, 450, 482], "Revenue 2021-2025 (USD M)"),
            width=120 * mm,
            height=71 * mm,
        ),
        Paragraph("Figure 2. Total revenue from 2021 to 2025.", body),
        PageBreak(),
        Paragraph("4. Risks and Outlook", ss["Heading2"]),
        Paragraph(
            "The main risk identified by the board is supply chain concentration: about "
            "40 percent of motor controllers come from a single supplier in Taiwan. "
            "Management has started qualifying two additional suppliers and expects this "
            "to be completed by the end of 2026.",
            body,
        ),
        Paragraph(
            "For 2026 Aurora Robotics guides to revenue between 540 and 560 million US "
            "dollars and plans to open a customer experience center in Rotterdam. Research "
            "and development spending will rise to 16 percent of revenue, focused on "
            "fleet orchestration software and battery efficiency.",
            body,
        ),
        PageBreak(),
        Paragraph("5. Sustainability", ss["Heading2"]),
        Paragraph(
            "The Gimhae plant runs on 62 percent renewable electricity, and the company "
            "targets 100 percent by 2028. Packaging waste per robot fell by 23 percent "
            "after the introduction of reusable transport frames.",
            body,
        ),
    ]
    doc.build(story)


def korean(out: Path) -> None:
    pdfmetrics.registerFont(TTFont("Nanum", str(FONTS / "NanumGothic-Regular.ttf")))
    pdfmetrics.registerFont(TTFont("NanumBold", str(FONTS / "NanumGothic-Bold.ttf")))
    title = ParagraphStyle("t", fontName="NanumBold", fontSize=20, leading=26, spaceAfter=12)
    h = ParagraphStyle("h", fontName="NanumBold", fontSize=14, leading=20, spaceBefore=10, spaceAfter=6)
    p = ParagraphStyle("p", fontName="Nanum", fontSize=11, leading=17)
    doc = SimpleDocTemplate(str(out), pagesize=A4, title="오로라 로보틱스 2025 연간 보고서")
    story = [
        Paragraph("오로라 로보틱스 2025 연간 보고서", title),
        Paragraph("1. 요약", h),
        Paragraph(
            "오로라 로보틱스는 2012년 부산에서 설립된 물류 자동화 로봇 기업입니다. "
            "2025년 총매출은 4억 8,200만 달러로 전년 대비 18퍼센트 증가했습니다. "
            "박민준 대표이사는 자율 피킹 시스템 수요와 서비스 사업 확대를 성장 요인으로 꼽았습니다.",
            p,
        ),
        Paragraph("2. 지역별 매출", h),
        Paragraph(
            "아시아 태평양 지역이 31퍼센트로 가장 빠르게 성장했고, 북미가 17퍼센트로 뒤를 이었습니다.",
            p,
        ),
        grid(
            [
                ["지역", "2024년(백만 달러)", "2025년(백만 달러)", "성장률"],
                ["아시아 태평양", "152", "199", "31%"],
                ["북미", "121", "142", "17%"],
                ["유럽", "105", "114", "9%"],
                ["중동", "30", "27", "-10%"],
            ],
            "Nanum",
            [38 * mm, 40 * mm, 40 * mm, 24 * mm],
        ),
        Spacer(1, 6 * mm),
        RLImage(
            bar_chart([199, 142, 114, 27], ["APAC", "NA", "EU", "ME"], "2025 revenue (USD M)"),
            width=110 * mm,
            height=65 * mm,
        ),
        Paragraph("그림 1. 2025년 지역별 매출", p),
        PageBreak(),
        Paragraph("3. 위험과 전망", h),
        Paragraph(
            "이사회가 꼽은 주요 위험은 공급망 집중입니다. 모터 컨트롤러의 약 40퍼센트를 대만의 "
            "단일 공급사에서 조달하고 있으며, 2026년 말까지 두 곳의 추가 공급사 인증을 마칠 계획입니다.",
            p,
        ),
        Paragraph(
            "2026년 매출 가이던스는 5억 4,000만 달러에서 5억 6,000만 달러이며, 로테르담에 "
            "고객 체험 센터를 열 예정입니다.",
            p,
        ),
        PageBreak(),
        Paragraph("4. 지속가능성", h),
        Paragraph(
            "김해 공장의 재생에너지 비율은 62퍼센트이며 2028년까지 100퍼센트를 목표로 합니다.",
            p,
        ),
    ]
    doc.build(story)


def unit_figure(k: int) -> io.BytesIO:
    """Distinct picture per unit: the label 'UNIT kk' exists only as pixels, never in the text."""
    img = Image.new("RGB", (480, 280), "white")
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, 479, 50], fill=PALETTE[k % len(PALETTE)])
    d.text((16, 18), f"UNIT {k:02d}", fill="white", font_size=26)
    for i in range(5):
        h = 40 + ((k * 37 + i * 53) % 150)
        d.rectangle([40 + i * 85, 260 - h, 100 + i * 85, 259], fill=PALETTE[(k + i) % len(PALETTE)])
    return _jpeg(img)


def gallery(out: Path) -> None:
    ss = getSampleStyleSheet()
    body = ss["BodyText"]
    doc = SimpleDocTemplate(str(out), pagesize=A4, title="Aurora Robotics Sensor Atlas")
    story: list = [
        Paragraph("Aurora Robotics Sensor Atlas", ss["Title"]),
        Paragraph(
            "This atlas lists the 28 sensor units installed in the Gimhae plant. Every unit is "
            "shown with a status chart; the unit number appears only in the picture.",
            body,
        ),
    ]
    for k in range(1, 29):
        story.append(Paragraph(f"Unit group {k}", ss["Heading3"]))
        story.append(
            Paragraph(
                f"The unit in bay {k} monitors a zone of {10 + k} meters and reports every "
                f"{2 + k % 5} seconds to the fleet controller.",
                body,
            )
        )
        story.append(RLImage(unit_figure(k), width=70 * mm, height=41 * mm))
        story.append(Paragraph(f"Figure {k}. Status chart of the unit in bay {k}.", body))
    doc.build(story)


# ---- analyze (vision) fixtures: one image per PDF -----------------------------------------------
FONT_KO = str(FONTS / "NanumGothic-Regular.ttf")
FONT_KO_BOLD = str(FONTS / "NanumGothic-Bold.ttf")


def _font(size: int, ko: bool = False, bold: bool = False):
    from PIL import ImageFont

    return ImageFont.truetype(FONT_KO_BOLD if bold else FONT_KO, size) if ko else ImageFont.load_default(size)


def vision_chart() -> Image.Image:
    """Grouped bar chart with axis, gridlines, legend and value labels (the answers are the labels)."""
    img = Image.new("RGB", (900, 560), "white")
    d = ImageDraw.Draw(img)
    d.text((30, 14), "Quarterly Revenue (USD M)", fill="black", font=_font(28))
    left, bottom, top = 90, 480, 90
    for v in (0, 50, 100, 150, 200, 250):
        y = bottom - (bottom - top) * v / 250
        d.line([left, y, 860, y], fill="#CCCCCC", width=1)
        d.text((40, y - 9), str(v), fill="black", font=_font(18))
    d.line([left, top, left, bottom], fill="black", width=2)
    d.line([left, bottom, 860, bottom], fill="black", width=2)
    series = {"2024": [100, 130, 150, 170], "2025": [120, 150, 170, 200]}
    colors_ = {"2024": "#9AA5B1", "2025": "#3E63DD"}
    for qi, q in enumerate(["Q1", "Q2", "Q3", "Q4"]):
        x0 = left + 40 + qi * 180
        for si, (name, vals) in enumerate(series.items()):
            v = vals[qi]
            x = x0 + si * 70
            y = bottom - (bottom - top) * v / 250
            d.rectangle([x, y, x + 58, bottom - 1], fill=colors_[name])
            d.text((x + 8, y - 24), str(v), fill="black", font=_font(20))
        d.text((x0 + 45, bottom + 8), q, fill="black", font=_font(20))
    for i, (name, col) in enumerate(colors_.items()):
        d.rectangle([680 + i * 90, 22, 700 + i * 90, 42], fill=col)
        d.text((706 + i * 90, 20), name, fill="black", font=_font(20))
    return img


def vision_table() -> Image.Image:
    rows = [["Region", "2024", "2025", "Growth"], ["Asia-Pacific", "152", "199", "31%"], ["North America", "121", "142", "17%"], ["Europe", "105", "114", "9%"], ["Middle East", "30", "27", "-10%"]]
    img = Image.new("RGB", (860, 400), "white")
    d = ImageDraw.Draw(img)
    widths = [300, 170, 170, 170]
    y = 20
    for r, row in enumerate(rows):
        x = 20
        for c, cell in enumerate(row):
            d.rectangle([x, y, x + widths[c], y + 66], outline="black", width=2, fill="#E8EAF0" if r == 0 else "white")
            d.text((x + 14, y + 18), cell, fill="black", font=_font(28))
            x += widths[c]
        y += 66
    return img


def vision_table_photo() -> Image.Image:
    """The table above 'photographed' on a wooden desk: rotated, shadowed, noisy (not a clean grid,
    so the extractor treats it as a picture instead of a table)."""
    import random

    from PIL import ImageFilter

    t = vision_table().convert("RGBA")
    bg = Image.new("RGB", (1100, 640), (176, 142, 104))
    px = bg.load()
    rnd = random.Random(3)
    for y in range(640):
        for x in range(1100):
            n = rnd.randint(-14, 14)
            r, g, b = px[x, y]
            px[x, y] = (max(0, min(255, r + n + (x // 90) % 3 * 4)), max(0, min(255, g + n)), max(0, min(255, b + n)))
    shadow = Image.new("RGBA", (t.width + 30, t.height + 30), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rectangle([15, 15, t.width + 15, t.height + 15], fill=(0, 0, 0, 110))
    shadow = shadow.filter(ImageFilter.GaussianBlur(10))
    shadow.paste(t, (0, 0), t)
    rot = shadow.rotate(3, expand=True, resample=Image.BICUBIC)
    bg.paste(rot, (80, 40), rot)
    return bg.filter(ImageFilter.GaussianBlur(0.6))


def vision_text() -> Image.Image:
    """Korean poster: headline plus detail lines (OCR of Hangul)."""
    img = Image.new("RGB", (900, 700), "#FFF8E1")
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, 899, 120], fill="#D84315")
    d.text((40, 30), "2026 로봇 박람회", fill="white", font=_font(56, ko=True, bold=True))
    lines = [
        "일시: 2026년 3월 14일(토) 오전 10시",
        "장소: 부산 벡스코 제2전시장 3홀",
        "주최: 오로라 로보틱스",
        "입장료: 무료 (사전 등록 필수)",
        "문의: 051-123-4567",
        "주차는 제1전시장 지하 주차장을 이용하세요",
    ]
    for i, line in enumerate(lines):
        d.text((50, 170 + i * 80), line, fill="#222222", font=_font(34, ko=True))
    d.rectangle([0, 0, img.width - 1, img.height - 1], outline="#5D4037", width=14)
    for i, c in enumerate(["#D84315", "#FFB300", "#0277BD"]):
        d.ellipse([700 + i * 50, 560, 780 + i * 50, 640], fill=c)
    return img


def vision_photo() -> Image.Image:
    """Photo-like landscape: sky gradient, sun, mountains, a lake and a red-roofed house."""
    img = Image.new("RGB", (800, 520))
    px = img.load()
    for y in range(520):
        for x in range(800):
            if y < 300:
                t = y / 300
                px[x, y] = (int(110 + 120 * t), int(170 + 70 * t), int(235 - 20 * t))
            else:
                t = (y - 300) / 220
                px[x, y] = (int(40 + 20 * t), int(110 - 30 * t), int(170 - 40 * t))
    d = ImageDraw.Draw(img)
    d.ellipse([600, 50, 700, 150], fill="#FFD54F")
    d.polygon([(0, 300), (150, 130), (300, 300)], fill="#6D6D78")
    d.polygon([(200, 300), (400, 90), (620, 300)], fill="#55555F")
    d.polygon([(130, 150), (150, 130), (172, 160)], fill="white")
    d.polygon([(380, 120), (400, 90), (424, 128)], fill="white")
    d.rectangle([330, 330, 450, 400], fill="#C9B79C")
    d.polygon([(320, 330), (390, 280), (460, 330)], fill="#B71C1C")
    d.rectangle([375, 360, 405, 400], fill="#5D4037")
    d.line([(0, 330), (800, 330)], fill="#3B6E4F", width=3)
    return img


def vision_small() -> Image.Image:
    img = Image.new("RGB", (150, 110), "white")
    d = ImageDraw.Draw(img)
    d.rectangle([20, 50, 55, 100], fill="#3E63DD")
    d.rectangle([80, 20, 115, 100], fill="#E5484D")
    d.text((24, 32), "30", fill="black", font=_font(16))
    d.text((84, 2), "70", fill="black", font=_font(16))
    d.text((26, 100), "A", fill="black", font=_font(10))
    d.text((88, 100), "B", fill="black", font=_font(10))
    return img


def single_image_pdf(out: Path, img: Image.Image, width_mm: float, caption: str, pagesize=A4) -> None:
    """One figure with a heading, an intro sentence and a caption: without that context the
    extractor's layout model often fails to treat a lone image as a picture."""
    buf = io.BytesIO()
    img.convert("RGB").save(buf, format="JPEG", quality=92)
    buf.seek(0)
    ss = getSampleStyleSheet()
    w = width_mm * mm
    h = w * img.height / img.width
    story: list = [
        Paragraph("Supplied material", ss["Heading2"]),
        Paragraph("The following figure was supplied by the organizer and is reproduced below.", ss["BodyText"]),
        Spacer(1, 4 * mm),
        RLImage(buf, width=w, height=h),
        Paragraph(caption, ss["BodyText"]),
    ]
    SimpleDocTemplate(str(out), pagesize=pagesize, title=out.stem).build(story)


def vision_set() -> None:
    single_image_pdf(HERE / "vision_chart.pdf", vision_chart(), 170, "Figure 1. Quarterly revenue by year.")
    single_image_pdf(HERE / "vision_table.pdf", vision_table_photo(), 170, "Figure 1. Photograph of the printed revenue table.")
    single_image_pdf(HERE / "vision_text.pdf", vision_text(), 150, "Figure 1. Event poster.")
    single_image_pdf(HERE / "vision_photo.pdf", vision_photo(), 150, "Figure 1. Landscape photograph.")
    single_image_pdf(HERE / "vision_small.pdf", vision_small(), 24, "Figure 1. Small chart.")


SENTENCES = [
    "The fleet controller assigns every pick task to the nearest idle robot and re-plans the route when an aisle is blocked.",
    "Operators can pause a zone from the dashboard, and the robots finish their current motion before they stop.",
    "Battery health is sampled every ten seconds, and a robot below 20 percent charge returns to the nearest charging bay.",
    "All firmware images are signed, and a robot refuses to boot an image whose signature does not match the fleet key.",
    "The operating margin of a deployment depends mainly on the utilization of the robots during the night shift.",
    "Customers receive a weekly report that lists throughput, error rates and the number of manual interventions.",
    "A calibration run takes about twelve minutes and must be repeated after every mechanical replacement.",
    "Log files are rotated daily and kept for ninety days unless the customer requests a longer retention period.",
    "The vision system detects damaged packaging and routes the affected item to a manual inspection station.",
    "Software updates are rolled out to five percent of the fleet first and to the remaining robots after 24 hours.",
    "Aurora Robotics provides remote support around the clock, with a guaranteed first response time of 30 minutes.",
    "Safety scanners reduce the speed of a robot to 0.3 meters per second when a person enters the warning field.",
]

SNIPPETS = [
    "def assign(task, robots):\n    idle = [r for r in robots if r.state == 'idle']\n    best = min(idle, key=lambda r: r.distance_to(task.pick_point))\n    best.start(task)\n    return best",
    "$ aurora-cli fleet status --zone B\nZONE  ROBOTS  IDLE  CHARGING\nB      42      11    5",
    "{\n  \"zone\": \"B\",\n  \"max_speed\": 1.2,\n  \"charge_threshold\": 0.2\n}",
]


def handbook(out: Path) -> None:
    """24-page handbook: 8 chapters of prose, a table, a code listing and (some) figures."""
    ss = getSampleStyleSheet()
    body = ss["BodyText"]
    code = ParagraphStyle("code", fontName="Courier", fontSize=8.5, leading=11, backColor=colors.whitesmoke)
    doc = SimpleDocTemplate(str(out), pagesize=A4, title="Aurora Robotics Fleet Handbook")
    story: list = [
        Paragraph("Aurora Robotics Fleet Handbook", ss["Title"]),
        Paragraph(
            "This handbook describes how to deploy, operate and maintain an Aurora Robotics "
            "warehouse fleet. It is written for operations engineers.",
            body,
        ),
    ]
    chapter_names = [
        "Fleet Architecture",
        "Installation",
        "Daily Operation",
        "Safety",
        "Maintenance",
        "Monitoring and Reporting",
        "Troubleshooting",
        "Upgrades and Support",
    ]
    for c, name in enumerate(chapter_names, start=1):
        story.append(PageBreak())
        story.append(Paragraph(f"{c}. {name}", ss["Heading1"]))
        for sec in range(1, 5):
            story.append(Paragraph(f"{c}.{sec} {name}: topic {sec}", ss["Heading2"]))
            for k in range(3):
                text = " ".join(SENTENCES[(c * 5 + sec * 3 + k * 7 + i) % len(SENTENCES)] for i in range(5))
                story.append(Paragraph(text, body))
            if sec == 2:
                rows = [["Parameter", "Default", "Unit"]] + [
                    [f"zone_{c}_{i}", str(round(0.4 * i + c, 1)), "m/s" if i % 2 else "percent"] for i in range(1, 5)
                ]
                story += [Spacer(1, 3 * mm), grid(rows, "Helvetica", [50 * mm, 35 * mm, 35 * mm]), Spacer(1, 3 * mm)]
            if sec == 3:
                from reportlab.platypus import Preformatted

                story += [Preformatted(SNIPPETS[c % len(SNIPPETS)], code), Spacer(1, 3 * mm)]
            if c in (1, 4, 6) and sec == 4:
                story.append(
                    RLImage(
                        bar_chart([40 + c * 3, 55, 31 + c, 72], ["A", "B", "C", "D"], f"Chart for chapter {c}"),
                        width=100 * mm,
                        height=59 * mm,
                    )
                )
                story.append(Paragraph(f"Figure {c}. Example chart for chapter {c}.", body))
    doc.build(story)


if __name__ == "__main__":
    vision_set()
    for name, fn in [("report_en.pdf", english), ("report_ko.pdf", korean), ("handbook_en.pdf", handbook), ("gallery_en.pdf", gallery)]:
        target = HERE / name
        fn(target)
        print(f"wrote {target} ({target.stat().st_size} bytes)")
