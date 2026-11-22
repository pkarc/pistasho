"""How a person types a question at a prompt, applied to the training and evaluation questions.

Models write questions like prose: a capital letter, accents, "¿...?", a full stop. Nobody types that
into a command line (an accent costs a dead key; the capital a shift), and a model trained on the
formal spelling is tested on questions that do not look like it. So, as a person would:

- all lowercase, no accents on the vowels (the ñ has a key of its own and stays),
- no ¿ or ¡, no exclamation marks, no full stop at the end,
- commas stay, and a closing "?" is sometimes there, as in English.

Applied with a probability (`casual`), not always: some people do type it properly, and the
evaluation questions written by hand mostly do.
"""
import random
import re

_ACCENTS = str.maketrans("áéíóúüÁÉÍÓÚÜ", "aeiouuaeiouu")


def casual_text(text, keep_question_mark=False):
    """The lowercase, unaccented, unpunctuated form of one question."""
    t = text.replace("¿", "").replace("¡", "").replace("!", "")
    t = t.translate(_ACCENTS).lower()
    t = " ".join(t.split())
    t = t.rstrip(" .:;?").strip()
    if keep_question_mark:
        t += "?"
    return t


def casual(text, rng, p=0.8):
    """`casual_text` with probability `p` (and a closing "?" in about a quarter of those), else untouched."""
    if rng.random() >= p:
        return text
    return casual_text(text, keep_question_mark=rng.random() < 0.25)


if __name__ == "__main__":
    # a few checks by hand: python3 training/style.py
    assert casual_text("¿Cómo hago para ver los archivos ocultos?") == "como hago para ver los archivos ocultos"
    assert casual_text("Reduce screen brightness by 10 percent.") == "reduce screen brightness by 10 percent"
    assert casual_text("Quiero ver el nombre del año, rápido!") == "quiero ver el nombre del año, rapido"
    assert casual_text("Borra el PDF", keep_question_mark=True) == "borra el pdf?"
    assert casual_text("  Mostrar   los  DNS  ") == "mostrar los dns"
    rng = random.Random(1)
    out = [casual("¿Cómo creo una carpeta?", rng, 0.8) for _ in range(1000)]
    changed = sum(o != "¿Cómo creo una carpeta?" for o in out)
    assert 750 < changed < 850 and all(o.islower() or o.startswith("¿") for o in out)
    print("ok")
