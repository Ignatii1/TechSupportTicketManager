# Task card — template

A card is everything a worker needs and nothing else: it starts cold and will not ask twice.

```
# Card <round>-<n>: <one behaviour, in the user's words>

Goal.        What exists after this card that didn't before (2–3 lines).
Oracle.      Where the truth is: C# file and lines (rule by rule), the C# check whose samples to port, the HISTORY
             entry, a server sample. What deliberately differs from the oracle, if anything, and why.
API.         The signatures the code will expose — names and types, fixed by the lead.
Files.       tester: <test file> (+ stubs in <product file>); builder: <product files>. Not to touch: <…>.
Read first.  docs/map/<area>.md § <…>; neighbours to imitate: <a product file>, <its test file>.
Acceptance.  <the gate command with the card's targets> is green; the checks cover: <the items of the round's check
             list that this card owns>.
Out of scope. What looks related but belongs to another card or to the lead.
```

## Example (layer 3.2, the search form's dates)

```
# Card 3.2-1: dates of the search form — «дд.мм.гггг» into a period

Goal.        The search form takes dates as the user types them; a pure function turns the two fields into the period
             the list query needs, or says which field is wrong.
Oracle.      TicketBoard/Services/TicketSearch.cs — `Period` and `TryDate` (rules: empty = open end, the «по» day is
             inclusive, the message names the field). Samples: TicketSearch.SelfCheck, the block «даты».
API.         rust/src/search.rs:
             pub fn period(from: &str, to: &str) -> Result<(Option<NaiveDate>, Option<NaiveDate>), String>
Files.       tester: rust/tests/search.rs (+ a stub in rust/src/search.rs, `pub mod search;` in rust/src/lib.rs);
             builder: rust/src/search.rs. Not to touch: rust/src/intraservice/query.rs.
Read first.  docs/map/search.md; neighbours: rust/src/refresh.rs, rust/tests/refresh.rs.
Acceptance.  python3 tools/gate.py --rust --quick --tests search is green; the checks cover items 1–2 of the round's
             check list (valid, empty, one-sided, reversed, not a date, 29 February).
Out of scope. Sending the query; the window.
```

## Briefing a reviewer

```
Lens: parity.   Diff: git diff <base>..HEAD -- rust/src/search.rs rust/tests/search.rs
Card: <the card, as given to the workers>
```
