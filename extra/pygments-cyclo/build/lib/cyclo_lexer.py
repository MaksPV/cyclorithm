"""Pygments lexer for the Cyclorithm DSL (``*.cyclo``).

Token set mirrors ``crates/cyclorithm-parser/src/grammar.pest``:
declaration keywords (``schedule``/``point``/``cycle``/``routine``/``root_cycle``),
declaration keywords (``const``/``fun``/``pred``/``time_const``), cycle modifiers
(``repeat``/``fill``/``until``/``gaps``), logic words (``and``/``or``/``not``,
``true``/``false``, ``floordiv``/``floormod``), ``//`` comments, ``"..."``
strings, durations (``-4h15m``, ``50m``, ``1ms``) and identifiers.
"""

from pygments.lexer import RegexLexer, bygroups, words
from pygments.token import Comment, Keyword, Name, Number, Operator, Punctuation, String, Text

__all__ = ["CycloLexer"]


class CycloLexer(RegexLexer):
    """Lexer for ``.cyclo`` schedule files."""

    name = "cyclo"
    aliases = ["cyclo"]
    filenames = ["*.cyclo"]
    url = "https://github.com/MaksPV/cyclorithm"

    # Declaration keywords: kw_schedule/kw_use/kw_const/kw_fun/kw_pred/
    # kw_time_const/kw_point/kw_actions/kw_attrs/kw_cycle/kw_routine/
    # kw_root_cycle/kw_start_time/kw_duration/kw_reverse.
    DECLARATION_KEYWORDS = (
        "schedule",
        "use",
        "const",
        "fun",
        "pred",
        "time_const",
        "point",
        "actions",
        "attrs",
        "cycle",
        "routine",
        "root_cycle",
        "start_time",
        "duration",
        "reverse",
    )

    # Cycle modifiers (repeat_kw/fill_kw/until_kw/kw_gaps) and condition
    # logic (kw_and/kw_or/kw_not, floordiv/floormod operators).
    MODIFIER_KEYWORDS = (
        "repeat",
        "fill",
        "until",
        "gaps",
        "and",
        "or",
        "not",
        "floordiv",
        "floormod",
    )

    tokens = {
        "root": [
            (r"\s+", Text),
            (r"//.*?$", Comment.Single),
            (r'"', String, "string"),
            (words(DECLARATION_KEYWORDS, suffix=r"\b"), Keyword),
            (words(MODIFIER_KEYWORDS, suffix=r"\b"), Keyword),
            (words(("true", "false"), suffix=r"\b"), Keyword.Constant),
            # Slot labels: 2nd, 3rd (label may start with a digit, see grammar).
            (r"\d+(?:st|nd|rd|th)\b", Name.Label),
            # Durations: 1h20m, -4h15m, 50m, 1ms, -0m (ms before m, as in the
            # grammar; items may sit back-to-back, so no trailing guard).
            (r"-?\d+(?:\.\d+)?(?:ms|[smhdw])", Number),
            (r"-?\d+(?:\.\d+)?(?![A-Za-z0-9_])", Number),
            # Calls: Name.action( — point/routine reference plus action.
            (r"([A-Za-z_][A-Za-z0-9_]*)(\s*)(\()",
             bygroups(Name.Function, Text, Punctuation)),
            (r"(\.)([A-Za-z_][A-Za-z0-9_]*)", bygroups(Punctuation, Name.Function)),
            (r"[A-Za-z_][A-Za-z0-9_]*", Name),
            (r"[=<>!]=|==|<<|>>|[-+*/%<>|&^!=]", Operator),
            (r"[:;,.\(\)\{\}\[\]]", Punctuation),
        ],
        "string": [
            (r'"', String, "#pop"),
            (r"\\.", String.Escape),
            (r'[^"\\]+', String),
        ],
    }
