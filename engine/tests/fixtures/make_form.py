"""Generates form.pdf, a small AcroForm used by tests/fill.rs.

The PDF is produced by reportlab (a different writer than the lopdf-built forms in the tests),
so it exercises a real-world field structure. Run with the extractor's virtualenv:

    extractor/.venv/bin/python engine/tests/fixtures/make_form.py
"""

from pathlib import Path

from reportlab.lib.pagesizes import A4
from reportlab.pdfgen import canvas

out = Path(__file__).with_name("form.pdf")
c = canvas.Canvas(str(out), pagesize=A4, invariant=1)  # invariant: reproducible bytes
form = c.acroForm

c.setFont("Helvetica", 12)
c.drawString(50, 780, "Application form")

c.drawString(50, 740, "Full name")
form.textfield(name="full_name", x=160, y=730, width=250, height=22, borderStyle="inset", forceBorder=True)
c.drawString(50, 700, "Email")
form.textfield(name="Email Address", x=160, y=690, width=250, height=22, borderStyle="inset", forceBorder=True)
c.drawString(50, 660, "Comments")
form.textfield(name="comments", x=160, y=600, width=250, height=50, fieldFlags="multiline", forceBorder=True)

c.drawString(50, 570, "I agree")
form.checkbox(name="agree_terms", x=160, y=565, size=16, buttonStyle="check", checked=False, forceBorder=True)

c.drawString(50, 530, "Plan")
form.radio(name="plan", tooltip="basic", value="basic", x=160, y=525, size=14, selected=False, forceBorder=True)
form.radio(name="plan", tooltip="pro", value="pro", x=220, y=525, size=14, selected=False, forceBorder=True)

c.drawString(50, 490, "Country")
form.choice(
    name="country",
    value="KR",
    # reportlab writes each tuple (a, b) as the PDF pair [b, a]; the PDF spec wants
    # [export value, display text], so the tuples are given as (display text, export value).
    options=[("Korea", "KR"), ("United States", "US"), ("Japan", "JP")],
    x=160,
    y=480,
    width=150,
    height=22,
    fieldFlags="combo",
    forceBorder=True,
)

c.showPage()
c.save()
print(f"wrote {out} ({out.stat().st_size} bytes)")
