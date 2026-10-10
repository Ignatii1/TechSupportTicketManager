"""Окно настроек (SettingsViewModel) — ловит SettingsCheck."""

AREAS = ['settings']

SETTINGSVIEWMODEL = 'TicketBoard/ViewModels/SettingsViewModel.cs'

MUTANTS = [
    ('range lower bound off', SETTINGSVIEWMODEL,
     'RangeError(value, 1, 50)',
     'RangeError(value, 0, 50)'),
    ('regex group not required', SETTINGSVIEWMODEL,
     'GetGroupNumbers().Length > 1',
     'GetGroupNumbers().Length >= 1'),
    ('http warning lost', SETTINGSVIEWMODEL,
     'BaseUrlWarning = BaseUrlError.Length == 0 && HttpIntraserviceClient.IsHttp(url) ? "По http пароль уходит открытым текстом — лучше https" : "";',
     'BaseUrlWarning = "";'),
    ('validity ignores the url', SETTINGSVIEWMODEL,
     ' + AutoSyncMinutesError + BaseUrlError).Length == 0;',
     ' + AutoSyncMinutesError).Length == 0;'),
    ('login saved untrimmed', SETTINGSVIEWMODEL,
     '_settings.IntraserviceLogin = Login.Trim();',
     '_settings.IntraserviceLogin = Login;'),
    ('empty field clears the password', SETTINGSVIEWMODEL,
     'if (newPassword.Length > 0) _settings.IntraservicePassword = newPassword;',
     '_settings.IntraservicePassword = newPassword;'),
    ('check ignores the saved password', SETTINGSVIEWMODEL,
     'var password = typedPassword.Length > 0 ? typedPassword : _settings.IntraservicePassword;',
     'var password = typedPassword;'),
    ('check hides the reason', SETTINGSVIEWMODEL,
     '$"Не удалось: {error}"',
     '"Не удалось"'),
    ('validity ignores one number', SETTINGSVIEWMODEL,
     ' + OverdueDaysError + AutoSyncMinutesError',
     ' + AutoSyncMinutesError'),
]
