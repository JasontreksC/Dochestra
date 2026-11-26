"""Generate the documents used by scripts/live-nodes.sh (committed output, <1 MB each).

    uv run --project extractor python fixtures/live/make_fixtures.py

Outputs (next to this file):
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


if __name__ == "__main__":
    for name, fn in [("report_en.pdf", english), ("report_ko.pdf", korean)]:
        target = HERE / name
        fn(target)
        print(f"wrote {target} ({target.stat().st_size} bytes)")
