"""Generates the realistic AcroForm fixtures used by tests/fill.rs and scripts/live/fill_check.sh:

  form_ko.pdf     Korean application form. Field names are a mix of Korean (이름, 생년월일, 연락처, 동의,
                  성별, 지역) and English (address, email, memo); the labels are only page text. The radio
                  exports are ASCII (male/female) while the labels beside the widgets are Korean (남/여), the
                  combo exports are ASCII with Korean display names - as in forms made by Korean offices.
  form_codes.pdf  The same form with meaningless field names (Text1, Text2, Check1, Group1, Dropdown1 ...):
                  the only hint about what a field means is the label printed next to it on the page.
  form_multi.pdf  Three pages: the applicant's name and phone are repeated on several pages as fields with the
                  same name (separate top-level fields, as some writers emit), five satisfaction questions
                  are radio groups with three widgets each, and labels sit above the fields on page 2.

The 지역 combo box is preset to its first option (서울): reportlab cannot make a combo without a default.
Labels use NanumGothic (engine/assets/fonts, SIL OFL), which reportlab subsets, so each PDF stays small.
Run with the extractor's virtualenv (reportlab is installed there; nothing is installed on the host):

    extractor/.venv/bin/python engine/tests/fixtures/make_forms.py
"""

from pathlib import Path

from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

# reportlab draws the preset value / option names of choice fields into a Latin-1 appearance stream and
# raises KeyError for Hangul. Only that placeholder appearance is affected (the /Opt strings are written
# as UTF-16), so characters outside Latin-1 are replaced by "?" there.
import reportlab.pdfbase.acroform as _acroform

_escape = _acroform.escPDF
_acroform.escPDF = lambda text: _escape("".join(c if ord(c) < 256 else "?" for c in text))

HERE = Path(__file__).resolve().parent
FONT = HERE.parent.parent / "assets" / "fonts" / "NanumGothic-Regular.ttf"
pdfmetrics.registerFont(TTFont("Nanum", str(FONT)))

# reportlab writes each choice tuple (a, b) as the PDF pair [b, a]; the PDF spec wants
# [export value, display text], so tuples are given as (display text, export value).
REGIONS = [("서울", "seoul"), ("부산", "busan"), ("대구", "daegu"), ("제주", "jeju")]
REGION_NAMES = ["서울", "부산", "대구", "제주"]


def canvas_for(name: str) -> canvas.Canvas:
    return canvas.Canvas(str(HERE / name), pagesize=A4, invariant=1)  # invariant: reproducible bytes


def text(form, label_canvas, label, name, y, *, x=170, width=240, multiline=False, label_x=50):
    label_canvas.setFont("Nanum", 12)
    label_canvas.drawString(label_x, y + 6, label)
    form.textfield(
        name=name,
        x=x,
        y=y,
        width=width,
        height=50 if multiline else 22,
        fieldFlags="multiline" if multiline else "",
        borderStyle="inset",
        forceBorder=True,
    )


def application(path: str, names: dict[str, str], choice_plain: bool) -> None:
    c = canvas_for(path)
    form = c.acroForm
    c.setFont("Nanum", 18)
    c.drawString(50, 790, "참가 신청서")
    c.setFont("Nanum", 10)
    c.drawString(50, 770, "* 모든 항목을 기재해 주세요.")

    text(form, c, "성명", names["name"], 730)
    text(form, c, "생년월일 (예: 1990-01-31)", names["birth"], 690, label_x=50, x=230, width=180)
    text(form, c, "주소", names["address"], 650)
    text(form, c, "연락처", names["phone"], 610)
    text(form, c, "이메일", names["email"], 570)

    c.setFont("Nanum", 12)
    c.drawString(50, 535, "성별")
    c.drawString(190, 535, "남")
    form.radio(name=names["gender"], value=names["male"], x=170, y=530, size=14, selected=False, forceBorder=True)
    c.drawString(250, 535, "여")
    form.radio(name=names["gender"], value=names["female"], x=230, y=530, size=14, selected=False, forceBorder=True)

    c.drawString(50, 495, "지역")
    form.choice(
        name=names["region"],
        value="서울" if choice_plain else "seoul",  # reportlab needs a default: the first option
        options=REGION_NAMES if choice_plain else REGIONS,
        x=170,
        y=485,
        width=150,
        height=22,
        fieldFlags="combo",
        forceBorder=True,
    )

    text(form, c, "비고", names["memo"], 410, multiline=True)

    c.setFont("Nanum", 12)
    c.drawString(190, 375, "개인정보 수집·이용에 동의합니다")
    form.checkbox(name=names["agree"], x=170, y=370, size=16, buttonStyle="check", checked=False, forceBorder=True)

    c.showPage()
    c.save()


def multi(path: str) -> None:
    c = canvas_for(path)
    form = c.acroForm

    # ---- page 1: applicant (label left of the field)
    c.setFont("Nanum", 18)
    c.drawString(50, 790, "행사 참가 신청서 (1/3)")
    text(form, c, "신청인 성명", "name", 740)
    text(form, c, "연락처", "phone", 700)
    text(form, c, "소속", "organization", 660)
    c.showPage()

    # ---- page 2: labels ABOVE the fields, name and phone repeated in the page header
    c.setFont("Nanum", 18)
    c.drawString(50, 790, "참가 확인 (2/3)")
    c.setFont("Nanum", 11)
    c.drawString(50, 760, "신청인 성명")
    form.textfield(name="name", x=50, y=728, width=200, height=22, borderStyle="inset", forceBorder=True)
    c.drawString(300, 760, "연락처")
    form.textfield(name="phone", x=300, y=728, width=200, height=22, borderStyle="inset", forceBorder=True)
    c.drawString(50, 690, "동반자 성명")
    form.textfield(name="companion", x=50, y=658, width=200, height=22, borderStyle="inset", forceBorder=True)
    c.drawString(300, 690, "참가일")
    form.textfield(name="visit_date", x=300, y=658, width=200, height=22, borderStyle="inset", forceBorder=True)
    c.drawString(50, 615, "식사 신청")
    form.checkbox(name="meal", x=50, y=590, size=16, buttonStyle="check", checked=False, forceBorder=True)
    c.drawString(300, 615, "주차 신청")
    form.checkbox(name="parking", x=300, y=590, size=16, buttonStyle="check", checked=False, forceBorder=True)
    c.showPage()

    # ---- page 3: five satisfaction questions, three widgets each
    c.setFont("Nanum", 18)
    c.drawString(50, 790, "만족도 조사 (3/3)")
    questions = ["프로그램 구성", "진행 방식", "장소", "안내 자료", "전반적 만족도"]
    c.setFont("Nanum", 11)
    c.drawString(300, 755, "1 (낮음)")
    c.drawString(360, 755, "2")
    c.drawString(410, 755, "3 (높음)")
    for i, question in enumerate(questions, start=1):
        y = 725 - (i - 1) * 40
        c.setFont("Nanum", 12)
        c.drawString(50, y + 4, question)
        for k, x in enumerate((300, 360, 420), start=1):
            form.radio(name=f"q{i}", value=str(k), x=x, y=y, size=14, selected=False, forceBorder=True)
    text(form, c, "연락처 확인", "phone", 480)
    text(form, c, "기타 의견", "comment", 400, multiline=True)
    c.showPage()
    c.save()


def main() -> None:
    application(
        "form_ko.pdf",
        dict(
            name="이름", birth="생년월일", address="address", phone="연락처", email="email",
            gender="성별", male="male", female="female", region="지역", memo="memo", agree="동의",
        ),
        choice_plain=False,
    )
    application(
        "form_codes.pdf",
        dict(
            name="Text1", birth="Text2", address="Text3", phone="Text4", email="Text5",
            gender="Group1", male="Choice1", female="Choice2", region="Dropdown1", memo="Text6", agree="Check1",
        ),
        choice_plain=True,
    )
    multi("form_multi.pdf")
    for name in ("form_ko.pdf", "form_codes.pdf", "form_multi.pdf"):
        p = HERE / name
        print(f"wrote {p} ({p.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
