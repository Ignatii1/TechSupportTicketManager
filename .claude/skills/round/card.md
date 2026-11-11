# Task card — template

A card is everything a worker needs and nothing else: it starts cold and will not ask twice.

```
# Card <round>-<n>: <one behaviour, in the user's words>

Goal.        What exists after this card that didn't before (2–3 lines).
Oracle.      Where the truth is: C# file and lines, the parity table that holds the C# code's answers (or the C#
             check whose samples to port), the HISTORY entry, a server sample. What deliberately differs, and why.
API.         The signatures the code will expose — names and types, fixed by the lead.
Files.       tester: <test file> (+ stubs in <product file>); builder: <product files>. Not to touch: <…>.
Read first.  docs/map/<area>.md § <…>; neighbours to imitate: <a product file>, <its test file>.
Acceptance.  <the gate command with the card's targets> is green; the checks cover: <the items of the round's check
             list that this card owns>.
Out of scope. What looks related but belongs to another card or to the lead.
```

## Example (layer 3.2, the search form's dates — the pilot card of this process)

```
# Card 3.2-1: dates of the search form — two fields into a period

Goal.        The Rust search form will take dates as the user types them. A pure function turns the fields «с» and
             «по» into the period the list query needs, or says which field is wrong — exactly as the C# app does.
Oracle.      TicketBoard/Services/TicketSearch.cs: `ParseSpan` (lines 153–162), `TryDate` (164–174), `DateFormats`
             (line 38). Expected answers are not derived by reading it: rust/tests/parity/search-dates.tsv holds the
             inputs with the answers the C# code gave. Check list: docs/HISTORY.md, 2026-11-11, «Пилот», items 1–7.
API.         rust/src/search.rs (new; `pub mod search;` in rust/src/lib.rs):
               pub fn parse_span(from: &str, to: &str, label: &str) -> Result<DateSpan, String>
             `DateSpan` = ticketboard::intraservice::query::DateSpan. A table answer is `OK <from> <to>`
             (`%Y-%m-%dT%H:%M:%S`, `-` for None) or `ERR <text>`.
Files.       tester: rust/tests/search.rs; rows of rust/tests/parity/search-dates.tsv (inputs only — the last cell
             is written by `dotnet run --project TicketBoard.SelfCheck -- parity --write`, never by hand); a stub in
             rust/src/search.rs and the `pub mod` line. builder: rust/src/search.rs.
             Not to touch: rust/tests/common/, rust/src/intraservice/, any C# file.
Read first.  rust/tests/parity.rs (how a table is read); neighbours: rust/src/refresh.rs, rust/tests/refresh.rs.
Acceptance.  python3 tools/gate.py --rust --quick --tests search parity   is green; the checks cover items 1–7;
             python3 tools/mutants.py --since <base> --file src/search.rs   leaves no survivor.
Out of scope. The form itself, the query URL (rust/src/intraservice/query.rs builds it), names → ids.
```

## Briefing a reviewer

```
Lens: parity.   Diff: git diff <base>..HEAD -- rust/src/search.rs rust/tests/search.rs rust/tests/parity/
Card: <the card, as given to the workers>
```
