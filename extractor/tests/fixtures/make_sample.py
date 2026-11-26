"""Generate tests/fixtures/sample.pdf (title, paragraphs, a table, a raster image).

Usage: uv run python tests/fixtures/make_sample.py
"""

import io
from pathlib import Path

from PIL import Image, ImageDraw
from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import getSampleStyleSheet
from reportlab.lib.units import mm
from reportlab.lib.utils import ImageReader
from reportlab.platypus import Image as RLImage
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer, Table, TableStyle

OUT = Path(__file__).with_name("sample.pdf")


def _figure() -> io.BytesIO:
    """A bar-chart-like raster figure (JPEG keeps the PDF small)."""
    img = Image.new("RGB", (600, 360), "white")
    draw = ImageDraw.Draw(img)
    draw.rectangle([40, 20, 580, 320], outline="black", width=2)
    palette = ["#3E63DD", "#E5484D", "#30A46C", "#F5A524", "#8E4EC6"]
    for i, h in enumerate([120, 200, 160, 240, 90]):
        x = 70 + i * 100
        draw.rectangle([x, 320 - h, x + 70, 319], fill=palette[i])
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    buf.seek(0)
    return buf


def main() -> None:
    styles = getSampleStyleSheet()
    doc = SimpleDocTemplate(str(OUT), pagesize=A4, title="Dochestra Sample Report")
    body = styles["BodyText"]

    table = Table(
        [
            ["Quarter", "Revenue", "Cost", "Profit"],
            ["Q1", "120", "80", "40"],
            ["Q2", "150", "90", "60"],
            ["Q3", "170", "100", "70"],
            ["Q4", "200", "110", "90"],
        ],
        colWidths=[35 * mm] * 4,
    )
    table.setStyle(
        TableStyle(
            [
                ("GRID", (0, 0), (-1, -1), 0.8, colors.black),
                ("BACKGROUND", (0, 0), (-1, 0), colors.lightgrey),
                ("FONTNAME", (0, 0), (-1, 0), "Helvetica-Bold"),
                ("ALIGN", (1, 1), (-1, -1), "RIGHT"),
            ]
        )
    )

    figure = ImageReader(_figure())
    story = [
        Paragraph("Annual Sales Report", styles["Title"]),
        Paragraph("1. Overview", styles["Heading2"]),
        Paragraph(
            "This report summarizes the sales performance of the company over the "
            "last four quarters. Revenue grew steadily while costs remained under "
            "control, resulting in a healthy increase in profit.",
            body,
        ),
        Spacer(1, 6 * mm),
        Paragraph("2. Quarterly Results", styles["Heading2"]),
        table,
        Spacer(1, 6 * mm),
        Paragraph("3. Revenue Chart", styles["Heading2"]),
        RLImage(figure.fileName, width=120 * mm, height=72 * mm),
        Paragraph("Figure 1. Revenue by product line.", body),
    ]
    doc.build(story)
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
