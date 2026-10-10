"""Окно «Поиск заявок» (SearchViewModel) — ловит SearchWindowCheck."""

AREAS = ['search']

SEARCHVIEWMODEL = 'TicketBoard/ViewModels/SearchViewModel.cs'

MUTANTS = [
    ('search list filled off the UI thread', SEARCHVIEWMODEL,
     'notFound: "ничего не найдено", detailed: true);\n        if (cts.IsCancellationRequested) return;',
     'notFound: "ничего не найдено", detailed: true).ConfigureAwait(false);\n        if (cts.IsCancellationRequested) return;'),
    ('v2 progress reports ignored', SEARCHVIEWMODEL,
     'new(m => { if (IsWorking && !run.IsCancellationRequested) WorkMessage = m; });',
     'new(m => { });'),
    ('filling guard (stale + wanted)', SEARCHVIEWMODEL,
     'if (_filling || e.PropertyName is not { } name) return;',
     'if (e.PropertyName is not { } name) return;'),
    ('wanted tracks the user choice', SEARCHVIEWMODEL,
     'case nameof(SelectedService) when SelectedService is { } s: _wanted = _wanted with { ServiceId = s.Id }; break;',
     ''),
    ('settings: same account keeps everything', SEARCHVIEWMODEL,
     'if (account == _account) return;',
     ''),
    ('cache ignores a changed ticket', SEARCHVIEWMODEL,
     '_texts.TryGetValue(row.Id, out var known) && known.Changed == changed',
     '_texts.TryGetValue(row.Id, out var known)'),
    ('search may restart while running', SEARCHVIEWMODEL,
     '[RelayCommand(AllowConcurrentExecutions = true)]\n    private async Task Search()',
     '[RelayCommand]\n    private async Task Search()'),
    ('matched names shown', SEARCHVIEWMODEL,
     'Matched = string.Join("\\n", resolved.Notes);',
     ''),
    ('clear marks the list stale', SEARCHVIEWMODEL,
     'IsStale = true;   // условия сброшены, а список на экране — по прежним',
     ''),
]
