//! Original RU/EN text and decimal GB formatting with exact integer threshold editing.
const BYTES_PER_GB: u64 = 1_000_000_000;
/// Settings retain the entire byte count. Nine fractional digits are sufficient and exact.
pub fn threshold(bytes: i64) -> String {
    let magnitude = bytes.unsigned_abs();
    let whole = magnitude / BYTES_PER_GB;
    let remainder = magnitude % BYTES_PER_GB;
    let sign = if bytes < 0 { "-" } else { "" };
    if remainder == 0 {
        return format!("{sign}{whole}");
    }
    let fraction = format!("{remainder:09}");
    format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
}
/// Decimal GB input is converted with integer arithmetic; precision finer than a byte is rejected.
pub fn parse_threshold(value: &str) -> Result<i64, String> {
    let invalid = || "Invalid nonnegative GB threshold or value exceeds the byte limit.".to_owned();
    let value = value.trim();
    let value = value.strip_prefix('+').unwrap_or(value);
    if value.is_empty() || value.starts_with('-') {
        return Err(invalid());
    }
    let separators = value.bytes().filter(|b| *b == b'.' || *b == b',').count();
    if separators > 1 {
        return Err(invalid());
    }
    let (whole, fraction) = value.split_once(['.', ',']).unwrap_or((value, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    if fraction.len() > 9 && fraction.as_bytes()[9..].iter().any(|b| *b != b'0') {
        return Err(invalid());
    }
    let whole = if whole.is_empty() {
        0
    } else {
        whole.parse::<u64>().map_err(|_| invalid())?
    };
    let fraction = &fraction[..fraction.len().min(9)];
    let fractional_bytes = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| invalid())? * 10_u64.pow(9 - fraction.len() as u32)
    };
    let bytes = whole
        .checked_mul(BYTES_PER_GB)
        .and_then(|n| n.checked_add(fractional_bytes))
        .ok_or_else(invalid)?;
    i64::try_from(bytes).map_err(|_| invalid())
}
/// Matches the original decimal N2 byte display, including half-away-from-zero rounding.
pub fn gb(bytes: i64, language: &str) -> String {
    let hundredths = (bytes.unsigned_abs() + 5_000_000) / 10_000_000;
    let whole = hundredths / 100;
    let fraction = hundredths % 100;
    let ru = language.starts_with("ru");
    let separator = if ru { '\u{a0}' } else { ',' };
    let decimal = if ru { ',' } else { '.' };
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(separator);
        }
        grouped.push(digit);
    }
    let sign = if bytes < 0 && hundredths != 0 {
        "-"
    } else {
        ""
    };
    let unit = if ru { "ГБ" } else { "GB" };
    format!("{sign}{grouped}{decimal}{fraction:02} {unit}")
}
pub fn text(language: &str, key: &str) -> String {
    TRANSLATIONS
        .iter()
        .find(|(candidate, _, _)| *candidate == key)
        .map_or_else(
            || key.to_owned(),
            |(_, ru, en)| {
                if language == "ru" {
                    (*ru).into()
                } else if language == "en" {
                    (*en).into()
                } else {
                    key.to_owned()
                }
            },
        )
}

// Imported verbatim from the original 0.2.2 WPF locale resources.
const TRANSLATIONS: &[(&str, &str, &str)] = &[
    (
        "Cleanup.Cancelled",
        "Операция отменена.",
        "Operation cancelled.",
    ),
    (
        "Cleanup.ConfirmationRequired",
        "Требуется отдельное подтверждение удаления.",
        "Deletion needs a separate confirmation.",
    ),
    (
        "Cleanup.InvalidSelection",
        "Выбор файлов не соответствует просмотренному плану.",
        "The selection does not match the reviewed plan.",
    ),
    (
        "Cleanup.PlanUnavailable",
        "План очистки устарел. Повторите анализ.",
        "The cleanup plan is stale. Analyze again.",
    ),
    (
        "Cleanup.Unavailable",
        "Данные для очистки недоступны.",
        "Cleanup data is unavailable.",
    ),
    ("All", r###"Все категории"###, r###"All categories"###),
    (
        "Settings.Invalid",
        r###"Исправьте неверные значения в настройках перед сохранением."###,
        r###"Correct invalid settings values before saving."###,
    ),
    (
        "Recommend.Observed",
        r###"Обнаруженные кэши программ"###,
        r###"Observed application caches"###,
    ),
    (
        "Recommend.Limit",
        r###"Подсказки для стандартных путей NuGet и pip, наблюдавшихся в проверке. Объём может быть неполным; проверяйте настройки программы. DiskBurrow не очищает эти папки."###,
        r###"Hints for standard NuGet and pip paths observed during this scan. Size may be incomplete; check the application's settings. DiskBurrow does not clear these folders."###,
    ),
    (
        "Action.Instructions",
        r###"Инструкция"###,
        r###"Instructions"###,
    ),
    (
        "Cleanup.OutcomeBound",
        r###"Показаны первые 2000 результатов; полный журнал сохраняется локально при успешной записи истории."###,
        r###"First 2000 outcomes displayed; the complete journal is stored locally when history persistence succeeds."###,
    ),
    (
        "Alert.Path",
        r###"Путь из уведомления"###,
        r###"Notification path"###,
    ),
    ("Nav.Overview", r###"Обзор"###, r###"Overview"###),
    (
        "Nav.Largest",
        r###"Крупные папки и файлы"###,
        r###"Largest folders and files"###,
    ),
    (
        "Nav.History",
        r###"История и изменения"###,
        r###"History and changes"###,
    ),
    ("Nav.Cleanup", r###"Очистка"###, r###"Cleanup"###),
    ("Nav.Settings", r###"Настройки"###, r###"Settings"###),
    ("Action.Scan", r###"Проверить сейчас"###, r###"Scan now"###),
    (
        "Action.Choose",
        r###"Выбрать папку"###,
        r###"Choose folder"###,
    ),
    (
        "Action.Export",
        r###"Экспорт отчёта"###,
        r###"Export report"###,
    ),
    ("Action.Cancel", r###"Отменить"###, r###"Cancel"###),
    ("Action.Analyze", r###"Анализировать"###, r###"Analyze"###),
    (
        "Action.Delete",
        r###"Удалить выбранные навсегда"###,
        r###"Permanently delete selected"###,
    ),
    (
        "Action.Save",
        r###"Сохранить настройки"###,
        r###"Save settings"###,
    ),
    ("Action.Open", r###"Открыть"###, r###"Open"###),
    ("Action.Exit", r###"Выйти"###, r###"Exit"###),
    (
        "Action.Pause",
        r###"Приостановить фоновые проверки"###,
        r###"Pause background scans"###,
    ),
    (
        "Action.Storage",
        r###"Настройки памяти Windows"###,
        r###"Windows Storage settings"###,
    ),
    (
        "Action.ExcludeFile",
        r###"Исключить файл"###,
        r###"Exclude file"###,
    ),
    (
        "Action.ExcludeCategory",
        r###"Исключить категорию"###,
        r###"Exclude category"###,
    ),
    ("Status.Ready", r###"Готово"###, r###"Ready"###),
    (
        "Status.Scanning",
        r###"Проверка файлов"###,
        r###"Scanning files"###,
    ),
    (
        "Status.Analyzing",
        r###"Анализ очистки"###,
        r###"Analyzing cleanup"###,
    ),
    (
        "Status.Cleaning",
        r###"Удаление выбранных файлов"###,
        r###"Deleting selected files"###,
    ),
    (
        "Status.Cancelled",
        r###"Операция отменена; завершённые результаты сохранены на экране"###,
        r###"Cancelled; completed results remain visible"###,
    ),
    (
        "Status.Exported",
        r###"Отчёт экспортирован"###,
        r###"Report exported"###,
    ),
    (
        "Status.Saved",
        r###"Настройки сохранены"###,
        r###"Settings saved"###,
    ),
    (
        "Status.Error",
        r###"Действие не выполнено. Подробности ниже."###,
        r###"Action failed. See details below."###,
    ),
    (
        "Status.Paused",
        r###"Фоновые проверки приостановлены"###,
        r###"Background scans paused"###,
    ),
    (
        "History.Error",
        r###"Не удалось прочитать или сохранить историю. При повреждении: выйдите из DiskBurrow, сохраните/переименуйте history.db и существующие history.db-journal/-wal/-shm вместе в %LOCALAPPDATA%\DiskBurrow; переместите копии базы и её служебных файлов вместе вне каталога данных с ограниченным бюджетом, оставьте settings.json и запустите приложение. Инструкция восстановления — в README."###,
        r###"Local history could not be read or saved. If corrupt: Exit DiskBurrow, preserve/rename history.db and existing history.db-journal/-wal/-shm together in %LOCALAPPDATA%\DiskBurrow; move the database/sidecar backups together outside the budgeted data directory, keep settings.json and restart. See README recovery instructions."###,
    ),
    (
        "Journal.Error",
        r###"Удаление завершено, но журнал не сохранён. Не повторяйте удаление."###,
        r###"Deletion finished, but its journal was not saved. Do not repeat deletion."###,
    ),
    (
        "Settings.Recovered",
        r###"Настройки не прочитаны; используются безопасные значения по умолчанию. Проверьте их перед сохранением."###,
        r###"Settings could not be read; safe defaults are in use. Review them before saving."###,
    ),
    (
        "Settings.TempRejected",
        r###"Дополнительный TEMP не прошёл проверку; правило для него отключено."###,
        r###"Custom TEMP failed validation; its cleanup rule is disabled."###,
    ),
    (
        "Settings.AutostartMoved",
        r###"Запись автозапуска указывает на другой EXE. Сохраните настройки, чтобы обновить путь."###,
        r###"Autostart points to another EXE. Save settings to update the path."###,
    ),
    (
        "Details",
        r###"Технические подробности"###,
        r###"Technical details"###,
    ),
    (
        "Empty",
        r###"Нажмите «Проверить сейчас», чтобы увидеть результат."###,
        r###"Choose Scan now to see a result."###,
    ),
    ("Root", r###"Проверяемая папка"###, r###"Scan root"###),
    (
        "Overview.Volume",
        r###"Объём тома по данным Windows"###,
        r###"Volume space reported by Windows"###,
    ),
    ("Overview.Used", r###"Занято"###, r###"Used"###),
    ("Overview.Free", r###"Свободно"###, r###"Free"###),
    (
        "Overview.Total",
        r###"Всего на томе"###,
        r###"Volume total"###,
    ),
    (
        "Overview.Checked",
        r###"Последняя проверка"###,
        r###"Last scan"###,
    ),
    (
        "Overview.Logical",
        r###"Размер доступных данных"###,
        r###"Accessible data size"###,
    ),
    (
        "Overview.Allocated",
        r###"Известный размер на диске"###,
        r###"Known allocated size"###,
    ),
    (
        "Overview.Coverage",
        r###"Покрытие проверки"###,
        r###"Scan coverage"###,
    ),
    (
        "Overview.Explanation",
        r###"Сумма файлов отличается от занятого места Windows: недоступные файлы и системные метаданные не входят в неё. Ссылки учитываются один раз. Проверка — наблюдение за интервал, а не атомарный снимок."###,
        r###"File totals differ from Windows used space: inaccessible files and system metadata are excluded. Hard links are counted once. A scan observes an interval, not an atomic snapshot."###,
    ),
    (
        "Coverage.Complete",
        r###"Проверенные поддеревья покрыты полностью"###,
        r###"Observed subtrees fully covered"###,
    ),
    (
        "Coverage.Incomplete",
        r###"Есть непроверенные или пропущенные области; выводы ограничены покрытием"###,
        r###"Some areas were inaccessible or skipped; conclusions are limited by coverage"###,
    ),
    ("Unknown", r###"Неизвестно"###, r###"Unknown"###),
    ("Yes", r###"Да"###, r###"Yes"###),
    ("No", r###"Нет"###, r###"No"###),
    ("Path", r###"Путь"###, r###"Path"###),
    ("Logical", r###"Данные"###, r###"Data"###),
    ("Allocated", r###"На диске"###, r###"Allocated"###),
    ("Coverage", r###"Покрытие"###, r###"Coverage"###),
    ("Modified", r###"Изменён"###, r###"Modified"###),
    ("Reason", r###"Причина"###, r###"Reason"###),
    ("Category", r###"Категория"###, r###"Category"###),
    ("Select", r###"Выбор"###, r###"Select"###),
    ("Filter", r###"Фильтр пути"###, r###"Path filter"###),
    ("Outcome", r###"Результат"###, r###"Outcome"###),
    ("Delta", r###"Изменение данных"###, r###"Data change"###),
    ("Kind", r###"Тип изменения"###, r###"Change kind"###),
    ("Comparable", r###"Сопоставимо"###, r###"Comparable"###),
    (
        "Scan.Time",
        r###"Время завершения"###,
        r###"Completed at"###,
    ),
    (
        "Lists.Bound",
        r###"На экране до 1000 папок, 100 файлов и 1000 изменений. Полный снимок сохранён для истории и экспорта. Суммы родительских и дочерних папок не складывайте."###,
        r###"Display limit: 1000 folders, 100 files and 1000 changes. The complete snapshot remains available for history and export. Do not add parent and child folder totals."###,
    ),
    (
        "History.Sparse",
        r###"Последние 30 завершённых проверок; до 100 крупнейших файлов в каждой. Выберите проверку для сравнения с предыдущей. Непокрытые области не считаются удалёнными."###,
        r###"Last 30 completed scans; up to 100 largest files each. Select a scan to compare with its predecessor. Uncovered areas are not treated as deleted."###,
    ),
    (
        "Cleanup.Warning",
        r###"Удаление постоянное. Корзина и откат не используются. Выберите точные файлы, затем подтвердите удаление. Ссылки могут не освободить указанный объём."###,
        r###"Deletion is permanent. No Recycle Bin or undo. Select exact files, then confirm deletion. Hard links may not release the estimated space."###,
    ),
    (
        "Cleanup.Confirm",
        r###"Навсегда удалить выбранные файлы? Это действие нельзя отменить."###,
        r###"Permanently delete selected files? This cannot be undone."###,
    ),
    (
        "Cleanup.Selected",
        r###"Выбрано: размер данных"###,
        r###"Selected: estimated data size"###,
    ),
    (
        "Cleanup.FreeDelta",
        r###"Изменение свободного места по данным Windows"###,
        r###"Observed Windows free-space change"###,
    ),
    (
        "Cleanup.Unattempted",
        r###"Выбранных файлов не обработано"###,
        r###"Selected files not attempted"###,
    ),
    (
        "Cleanup.Bound",
        r###"Показано до 2000 кандидатов. Фильтр применяется ко всему плану. Скрытый фильтром выбор сохраняется и входит в удаление; перед подтверждением проверьте число выбранных файлов."###,
        r###"Up to 2000 candidates displayed. Filtering covers the entire plan. Filtered-out selections remain selected and will be deleted; check the selected count before confirmation."###,
    ),
    (
        "Cleanup.Count",
        r###"Количество выбранных файлов"###,
        r###"Selected file count"###,
    ),
    (
        "Cleanup.TempAge",
        r###"Временный файл старше 7 суток"###,
        r###"Temporary file older than 7 days"###,
    ),
    (
        "Cleanup.OldTemp",
        r###"Временный файл старше 7 суток"###,
        r###"Temporary file older than 7 days"###,
    ),
    (
        "Cleanup.OldCrashDump",
        r###"Дамп старше 7 суток; может понадобиться для диагностики"###,
        r###"Crash dump older than 7 days; may be useful for diagnosis"###,
    ),
    (
        "Cleanup.BrowserCache",
        r###"HTTP-кэш закрытого браузера"###,
        r###"HTTP cache of a closed browser"###,
    ),
    (
        "Cleanup.UnsafeRoot",
        r###"Корень правила не прошёл проверку безопасности"###,
        r###"Rule root failed safety validation"###,
    ),
    (
        "Cleanup.NonLocalVolume",
        r###"Область не подтверждена как локальная"###,
        r###"Location is not confirmed local"###,
    ),
    (
        "Cleanup.KnownFoldersUnavailable",
        r###"Не удалось проверить расположение пользовательских библиотек"###,
        r###"User library locations could not be validated"###,
    ),
    (
        "Cleanup.TempApprovalRequired",
        r###"Для нестандартного TEMP нужно явное одобрение в настройках"###,
        r###"Custom TEMP requires explicit approval in Settings"###,
    ),
    (
        "Cleanup.OwnerRunning",
        r###"Закройте браузер перед анализом"###,
        r###"Close the browser before analysis"###,
    ),
    (
        "Cleanup.OwnerUnavailable",
        r###"Не удалось проверить процессы владельца"###,
        r###"Owner processes could not be checked"###,
    ),
    (
        "Cleanup.RootUnavailable",
        r###"Папка недоступна"###,
        r###"Folder unavailable"###,
    ),
    (
        "Cleanup.AncestorChanged",
        r###"Родительская папка изменилась после анализа"###,
        r###"Parent folder changed after analysis"###,
    ),
    (
        "Cleanup.UnsafeAncestor",
        r###"Родительская папка не прошла проверку"###,
        r###"Parent folder failed validation"###,
    ),
    (
        "Cleanup.UnsafePath",
        r###"Путь не прошёл проверку"###,
        r###"Path failed validation"###,
    ),
    (
        "Cleanup.VolumeChanged",
        r###"Том изменился после анализа"###,
        r###"Volume changed after analysis"###,
    ),
    (
        "Cleanup.FileChanged",
        r###"Файл изменился после анализа"###,
        r###"File changed after analysis"###,
    ),
    (
        "Cleanup.RuleUnavailable",
        r###"Правило больше недоступно"###,
        r###"Rule no longer available"###,
    ),
    (
        "Cleanup.TooRecent",
        r###"Файл слишком новый для правила"###,
        r###"File is too recent for this rule"###,
    ),
    (
        "Cleanup.UnsafeFile",
        r###"Файл не прошёл проверку безопасности"###,
        r###"File failed safety validation"###,
    ),
    (
        "Cleanup.FileUnavailable",
        r###"Метаданные файла недоступны"###,
        r###"File metadata unavailable"###,
    ),
    ("Outcome.Deleted", r###"Удалён"###, r###"Deleted"###),
    (
        "Outcome.Missing",
        r###"Уже отсутствует"###,
        r###"Already missing"###,
    ),
    (
        "Outcome.SkippedChanged",
        r###"Пропущен: изменился"###,
        r###"Skipped: changed"###,
    ),
    (
        "Outcome.SkippedBusy",
        r###"Пропущен: занят"###,
        r###"Skipped: busy"###,
    ),
    (
        "Outcome.SkippedPolicy",
        r###"Пропущен: правило не разрешает"###,
        r###"Skipped: policy"###,
    ),
    (
        "Outcome.Failed",
        r###"Не удалось удалить"###,
        r###"Deletion failed"###,
    ),
    ("Change.New", r###"Новая папка"###, r###"New folder"###),
    ("Change.Removed", r###"Удалена"###, r###"Removed"###),
    ("Change.Changed", r###"Изменилась"###, r###"Changed"###),
    (
        "Change.Unavailable",
        r###"Нет сопоставимого покрытия"###,
        r###"No comparable coverage"###,
    ),
    (
        "Issue.AccessDenied",
        r###"Отказ в доступе"###,
        r###"Access denied"###,
    ),
    (
        "Issue.ReparseSkipped",
        r###"Ссылка пропущена"###,
        r###"Reparse point skipped"###,
    ),
    (
        "Issue.CloudSkipped",
        r###"Облачная запись пропущена"###,
        r###"Cloud placeholder skipped"###,
    ),
    (
        "Issue.MetadataUnavailable",
        r###"Метаданные недоступны"###,
        r###"Metadata unavailable"###,
    ),
    (
        "Issue.ChangedDuringScan",
        r###"Изменилось во время проверки"###,
        r###"Changed during scan"###,
    ),
    (
        "Issue.IoFailure",
        r###"Ошибка чтения"###,
        r###"Read failure"###,
    ),
    (
        "Settings.Interval",
        r###"Интервал полной проверки, часов"###,
        r###"Full scan interval, hours"###,
    ),
    (
        "Settings.Low",
        r###"Предупреждать ниже, ГБ"###,
        r###"Warn below, GB"###,
    ),
    (
        "Settings.Growth",
        r###"Предупреждать о росте от, ГБ"###,
        r###"Warn about growth from, GB"###,
    ),
    (
        "Settings.Theme",
        r###"Тема оформления"###,
        r###"Appearance"###,
    ),
    ("Theme.Light", r###"Светлая"###, r###"Light"###),
    ("Theme.Dark", r###"Тёмная"###, r###"Dark"###),
    (
        "Settings.Battery",
        r###"Разрешить полные проверки на батарее"###,
        r###"Allow full scans on battery"###,
    ),
    (
        "Settings.Language",
        r###"Язык интерфейса"###,
        r###"Interface language"###,
    ),
    (
        "Settings.Exclusions",
        r###"Исключения очистки: полный путь на каждой строке"###,
        r###"Cleanup exclusions: one absolute path per line"###,
    ),
    (
        "Settings.Autostart",
        r###"Запускать с Windows (применяется только кнопкой сохранения)"###,
        r###"Start with Windows (applied only by Save settings)"###,
    ),
    (
        "Settings.Temp",
        r###"Явно одобренный нестандартный TEMP; пусто — отключён"###,
        r###"Explicitly approved custom TEMP; blank disables"###,
    ),
    (
        "Settings.Explanation",
        r###"Первая фоновая проверка через 5 минут. Свободное место — каждые 5 минут. Автозапуск выключен по умолчанию. Данные и история остаются локально."###,
        r###"First background scan after 5 minutes. Free space checked every 5 minutes. Autostart is off by default. Data and history remain local."###,
    ),
    (
        "Export.Disclosure",
        r###"Отчёт содержит личные пути файлов и папок. Сохранить его в выбранное вами место?"###,
        r###"The report contains personal file and folder paths. Save it to your chosen destination?"###,
    ),
    (
        "Recommend.Text",
        r###"Системные файлы очищайте штатными средствами Windows. Кэши других программ очищайте через их настройки; пользовательские проекты не считаются мусором."###,
        r###"Use Windows tools for system cleanup. Clear other application caches through their settings; user projects are not treated as junk."###,
    ),
    (
        "Recommend.Chrome",
        r###"Инструкция Chrome по очистке кэша"###,
        r###"Chrome cache instructions"###,
    ),
    (
        "Recommend.Edge",
        r###"Инструкция Edge по очистке кэша"###,
        r###"Edge cache instructions"###,
    ),
    (
        "Alert.Low",
        r###"Мало свободного места"###,
        r###"Low free space"###,
    ),
    (
        "Alert.Growth",
        r###"Папка выросла"###,
        r###"Folder growth"###,
    ),
    (
        "Tray.Hint",
        r###"DiskBurrow — наблюдение за диском"###,
        r###"DiskBurrow — disk monitoring"###,
    ),
    (
        "UserTemp",
        r###"Временные файлы пользователя"###,
        r###"User temporary files"###,
    ),
    ("CrashDumps", r###"Дампы сбоев"###, r###"Crash dumps"###),
    (
        "ChromeCache",
        r###"HTTP-кэш Chrome"###,
        r###"Chrome HTTP cache"###,
    ),
    (
        "EdgeCache",
        r###"HTTP-кэш Edge"###,
        r###"Edge HTTP cache"###,
    ),
    (
        "Link.Repository",
        r###"Проект на GitHub"###,
        r###"GitHub project"###,
    ),
    (
        "Link.Releases",
        r###"Загрузки и выпуски"###,
        r###"Downloads and releases"###,
    ),
    (
        "Cleanup.Missing",
        r###"Файл больше не существует."###,
        r###"File is no longer present."###,
    ),
    (
        "Cleanup.SkippedBusy",
        r###"Файл занят или доступ запрещён."###,
        r###"File is busy or access is denied."###,
    ),
    (
        "Cleanup.Failed",
        r###"Не удалось удалить файл."###,
        r###"File deletion failed."###,
    ),
    (
        "Status.Saving",
        r###"Сохранение настроек…"###,
        r###"Saving settings…"###,
    ),
    (
        "Status.Exporting",
        r###"Экспорт отчёта…"###,
        r###"Exporting report…"###,
    ),
    (
        "Overview.CountersObserved",
        r###"Счётчики тома проверены (отдельно от полного обхода):"###,
        r###"Volume counters observed at (independent of the full scan):"###,
    ),
    (
        "Alert.Unavailable",
        r###"Результат уведомления недоступен или удалён из истории. Проверка не запускалась."###,
        r###"The notified result is unavailable or expired. No scan was started."###,
    ),
    (
        "Alert.PathUnavailable",
        r###"Путь уведомления отсутствует в сохранённом результате."###,
        r###"The notified path is absent from the retained result."###,
    ),
    (
        "Alert.ComparisonUnavailable",
        r###"Результат уведомления показан; предыдущая проверка для сравнения удалена из истории."###,
        r###"The notified result is shown; its previous comparison has expired."###,
    ),
    (
        "Alert.Context",
        r###"Папка уведомления выбрана в соответствующем сохранённом результате."###,
        r###"Notified folder selected in the corresponding retained result."###,
    ),
    (
        "Pause.Explanation",
        r###"Пауза останавливает полные фоновые обходы. Проверки свободного места и предупреждения продолжаются."###,
        r###"Pause stops full background scans. Free-space checks and low-space alerts continue."###,
    ),
    (
        "Cleanup.Eligible",
        r###"Найдены подходящие файлы."###,
        r###"Eligible files found."###,
    ),
    (
        "Cleanup.HasWarnings",
        r###"Часть путей пропущена; см. предупреждения ниже."###,
        r###"Some locations were skipped; see warnings below."###,
    ),
    (
        "Cleanup.NoEligible",
        r###"Нет подходящих файлов; см. условия и предупреждения."###,
        r###"No eligible files; see eligibility rules and warnings."###,
    ),
    (
        "Cleanup.Scope",
        r###"Предпросмотр включает временные файлы пользователя старше 7 дней, дампы сбоев (.dmp) старше 7 дней и HTTP-кеш закрытых Chrome и Edge. Счётчики относятся к предпросмотру до фильтров и исключений. Данные других приложений не считаются мусором."###,
        r###"Preview covers user temporary files older than 7 days, crash dumps (.dmp) older than 7 days, and HTTP caches of closed Chrome and Edge browsers. Counts refer to the analyzed preview before filters and exclusions. Other application data is not treated as junk."###,
    ),
    (
        "Cleanup.Warnings",
        r###"Пропущенные пути и недоступные категории"###,
        r###"Skipped locations and unavailable categories"###,
    ),
    ("Issue.Count", r###"Количество"###, r###"Count"###),
    (
        "Manual.Analyze",
        r###"Просмотреть отмеченные папки и файлы"###,
        r###"Review marked folders and files"###,
    ),
    (
        "Manual.Delete",
        r###"Удалить просмотренное безвозвратно"###,
        r###"Delete reviewed selection permanently"###,
    ),
    (
        "Manual.Review",
        r###"Просмотр ручного удаления"###,
        r###"Review manual deletion"###,
    ),
    (
        "Manual.Selected",
        r###"Отмечено путей"###,
        r###"Marked paths"###,
    ),
    (
        "Manual.Roots",
        r###"Точные корни удаления (выбор родителя и потомка объединён)"###,
        r###"Exact reviewed roots (parent and child selections are combined)"###,
    ),
    ("Manual.Files", r###"Файлы"###, r###"Files"###),
    ("Manual.Folders", r###"Папки"###, r###"Folders"###),
    ("Manual.Close", r###"Закрыть"###, r###"Close"###),
    (
        "Manual.Result",
        r###"Результат ручного удаления"###,
        r###"Manual deletion result"###,
    ),
    (
        "Manual.Ready",
        r###"Просмотренный список готов. Отдельная кнопка удаления требует ещё одного подтверждения."###,
        r###"Reviewed inventory is ready. The separate Delete action requires another confirmation."###,
    ),
    (
        "Manual.Blocked",
        r###"Этот набор нельзя удалить. Устраните предупреждения и повторите просмотр."###,
        r###"This selection cannot be deleted. Resolve the warnings and review it again."###,
    ),
    (
        "Manual.PermanentWarning",
        r###"Ручное удаление безвозвратно. Без корзины и отмены. Включены просмотренные вложенные файлы папок; жёсткие ссылки могут не освободить место."###,
        r###"Manual deletion is permanent. No Recycle Bin or undo. Folder contents in the reviewed inventory are included; hard links may release no space."###,
    ),
    (
        "Manual.Confirm",
        r###"Безвозвратно удалить эти просмотренные папки и файлы?"###,
        r###"Permanently delete these reviewed folders and files?"###,
    ),
    (
        "Manual.Stale",
        r###"Удаление было выполнено или прервано. Снимок может устареть; повторите сканирование выбранной папки, чтобы обновить его."###,
        r###"Deletion was attempted. These scan results may be outdated; scan the selected root again to refresh them."###,
    ),
    (
        "Manual.SelectionHelp",
        r###"Отметьте конкретные папки или файлы, затем просмотрите набор. Смена отметок или контекста сканирования отменяет просмотр."###,
        r###"Mark exact folders or files, then review the selection. Changing marks or the scan context invalidates the review."###,
    ),
    (
        "Manual.ProtectedPath",
        r###"Путь защищён или не подходит для ручного удаления локальных пользовательских данных."###,
        r###"This path is protected or is not an eligible local user path."###,
    ),
    (
        "Manual.ConfirmationRequired",
        r###"Требуется явное подтверждение безвозвратного удаления."###,
        r###"Explicit confirmation of permanent deletion is required."###,
    ),
    (
        "Manual.PlanUnavailable",
        r###"Просмотренный план устарел или уже использован. Повторите просмотр выбранного."###,
        r###"The reviewed plan has expired or has already been consumed. Review the selection again."###,
    ),
    (
        "Manual.Changed",
        r###"Путь или содержимое изменились после просмотра; затронутый элемент сохранён."###,
        r###"The path or its contents changed after review; the affected item was preserved."###,
    ),
    (
        "Manual.Cancelled",
        r###"Операция отменена; этот элемент не удалён."###,
        r###"The operation was cancelled; this item was not deleted."###,
    ),
    (
        "Manual.UnsafePath",
        r###"Путь не прошёл проверку (ссылка, облачный placeholder или смена родителя)."###,
        r###"The path failed validation (link, cloud placeholder, or changed parent)."###,
    ),
    (
        "Manual.NotEmpty",
        r###"Остались новые или сохранённые элементы. Непустая папка не удалена."###,
        r###"New or preserved contents remain. The non-empty folder was not deleted."###,
    ),
    (
        "Manual.Unavailable",
        r###"Путь или его метаданные нельзя безопасно прочитать."###,
        r###"The location or its metadata could not be accessed safely."###,
    ),
    (
        "Map.AllocationNote",
        r###"Площадь: выбранная метрика. На диске: известная физическая память, жёсткие ссылки учитываются один раз; неизвестные размеры не рисуются. Количество: пути файлов, включая ссылки. Пустые объекты доступны через поиск."###,
        r###"Area follows the chosen metric. On disk: known physical allocation, hardlinks counted once; unknown sizes are not drawn. Count: file paths, including links. Empty items remain searchable."###,
    ),
    ("Map.Back", r###"Назад"###, r###"Back"###),
    ("Map.Class.App", r###"Программы"###, r###"Programs"###),
    ("Map.Class.Archive", r###"Архивы"###, r###"Archives"###),
    (
        "Map.Class.Disk",
        r###"Образы дисков"###,
        r###"Disk images"###,
    ),
    ("Map.Class.Document", r###"Документы"###, r###"Documents"###),
    ("Map.Class.Folder", r###"Папки"###, r###"Folders"###),
    ("Map.Class.Image", r###"Изображения"###, r###"Images"###),
    ("Map.Class.Media", r###"Медиа"###, r###"Media"###),
    ("Map.Class.Other", r###"Другие"###, r###"Other"###),
    (
        "Map.Covered",
        r###"Внутри отмеченной папки: чтобы снять отметку, снимите её с родительской папки."###,
        r###"Inside a marked folder: unmark its parent to remove the mark."###,
    ),
    (
        "Map.Enter",
        r###"Открыть папку на карте"###,
        r###"Enter folder"###,
    ),
    (
        "Map.Focus",
        r###"Выбранный объект"###,
        r###"Focused item"###,
    ),
    ("Map.Forward", r###"Вперёд"###, r###"Forward"###),
    ("Map.Global", r###"Весь диск"###, r###"Whole scan"###),
    (
        "Map.Historical",
        r###"История хранит ограниченный список крупных файлов, а не полный индекс. Запустите новое сканирование для карты диска."###,
        r###"History retains a bounded list of largest files, not the full index. Run a fresh scan to show the disk map."###,
    ),
    (
        "Map.Isolate",
        r###"Только совпадения"###,
        r###"Isolate matches"###,
    ),
    (
        "Map.Keys",
        r###"Двойной щелчок / Enter — открыть; Backspace / Esc — выше; Alt+←/→ — история. Ctrl+щелчок / пробел — отметить. Колесо — масштаб, средняя кнопка — перемещение, +/−/0 — масштаб."###,
        r###"Double-click / Enter: enter; Backspace / Esc: up; Alt+Left/Right: history. Ctrl+click / Space: mark. Wheel: zoom; middle drag: pan; +/−/0: zoom."###,
    ),
    (
        "Map.Legend",
        r###"Цвет обозначает тип файла; золотая рамка — совпадение, оранжевая — отметка. Пунктир — внутри отмеченной папки."###,
        r###"Color indicates file type; gold outline: match, orange: marked. Dashed outline: inside a marked folder."###,
    ),
    (
        "Map.Mark",
        r###"Отметить / снять"###,
        r###"Mark / unmark"###,
    ),
    (
        "Map.Matches",
        r###"Совпадения по имени"###,
        r###"Name matches"###,
    ),
    ("Map.Metric.Allocated", r###"На диске"###, r###"On disk"###),
    (
        "Map.Metric.Files",
        r###"Пути файлов"###,
        r###"File paths"###,
    ),
    (
        "Map.Metric.Logical",
        r###"Данные"###,
        r###"Logical bytes"###,
    ),
    (
        "Map.NoData",
        r###"Нет объектов с известным ненулевым размером для этой метрики. Попробуйте данные, пути файлов или поиск."###,
        r###"No entries with a known nonzero value for this metric. Try logical bytes, file paths, or search."###,
    ),
    (
        "Map.Others",
        r###"Остальные файлы"###,
        r###"Other entries"###,
    ),
    (
        "Map.RefreshVolumes",
        r###"Обновить диски"###,
        r###"Refresh drives"###,
    ),
    (
        "Map.Rescan",
        r###"Повторить сканирование"###,
        r###"Rescan"###,
    ),
    ("Map.Reset", r###"Масштаб 100%"###, r###"Reset zoom"###),
    ("Map.Root", r###"Корень"###, r###"Root"###),
    (
        "Map.Search",
        r###"Поиск по имени среди всех файлов текущего сканирования"###,
        r###"Search names across every file observed by the current scan"###,
    ),
    (
        "Map.SearchLimit",
        r###"Первые 1000 совпадений. Поиск охватывает весь живой индекс; двойной щелчок открывает папку."###,
        r###"First 1,000 matches shown. Search covers the full live index; double-click to navigate."###,
    ),
    (
        "Map.Selected",
        r###"Отмечен для отдельного просмотра перед удалением."###,
        r###"Marked for a separate review before deletion."###,
    ),
    (
        "Map.SelectedRoots",
        r###"Отмеченные пути"###,
        r###"Marked paths"###,
    ),
    ("Map.Unmark", r###"Снять"###, r###"Unmark"###),
    ("Map.Unselected", r###"Не отмечен."###, r###"Unmarked."###),
    ("Map.Up", r###"Выше"###, r###"Up"###),
    ("Map.VolumeFree", r###"Свободно"###, r###"Available free"###),
    (
        "Map.Volumes",
        r###"Локальные диски и свободное место; сначала наиболее заполненные. Выбор меняет корень сканирования."###,
        r###"Local drives and available free space, fullest first. Choosing one changes the scan root."###,
    ),
    ("Map.VolumeTotal", r###"Всего"###, r###"Total"###),
    ("Map.ZoomIn", r###"Увеличить"###, r###"Zoom in"###),
    ("Map.ZoomOut", r###"Уменьшить"###, r###"Zoom out"###),
    ("Nav.Map", r###"Карта диска"###, r###"Disk map"###),
    ("Map.Items", r###"Объекты папки"###, r###"Folder entries"###),
    (
        "Action.ScanFast",
        r###"Быстрый NTFS (UAC)"###,
        r###"Fast NTFS (UAC)"###,
    ),
    (
        "FastScan.Hint",
        r###"Только целый локальный NTFS-том. UAC запускает отдельный процесс чтения MFT. Основное окно и удаление работают без повышения прав. Живые метаданные могут отставать от недавних изменений."###,
        r###"Whole local NTFS volumes only. UAC starts a separate read-only MFT process. The main window and deletion stay unelevated. Live metadata may lag recent changes."###,
    ),
    (
        "FastScan.UacCancelled",
        "Запрос прав администратора отменён. Сканирование не запускалось.",
        "Administrator approval was cancelled. The scan did not start.",
    ),
    (
        "FastScan.ReadFailed",
        "Не удалось получить полный результат быстрого NTFS-сканирования.",
        "The fast NTFS reader could not produce a complete supported result.",
    ),
    ("Map.VolumeUsed", r###"Занято"###, r###"Used"###),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_preserve_every_byte_including_long_max() {
        for bytes in [
            0,
            1,
            17,
            999_999_999,
            1_000_000_000,
            5_368_709_120,
            16_106_127_360,
            i64::MAX - 1,
            i64::MAX,
        ] {
            assert_eq!(parse_threshold(&threshold(bytes)).unwrap(), bytes);
        }
        assert_eq!(threshold(16_106_127_360), "16.10612736");
        assert_eq!(threshold(1), "0.000000001");
        assert_eq!(threshold(i64::MAX), "9223372036.854775807");
    }
    #[test]
    fn parser_accepts_comma_without_float_rounding() {
        assert_eq!(parse_threshold(" 15,25 ").unwrap(), 15_250_000_000);
        assert_eq!(parse_threshold(".000000001").unwrap(), 1);
        assert_eq!(parse_threshold("1.").unwrap(), 1_000_000_000);
        assert_eq!(parse_threshold("0.0000000010").unwrap(), 1);
        for value in [
            "",
            ".",
            "-1",
            "-0",
            "NaN",
            "inf",
            "1e3",
            "1.2.3",
            "1,2.3",
            "1 000",
            "9223372036.854775808",
            "9223372037",
            "0.0000000001",
            "999999999999999999999999999999",
        ] {
            assert!(parse_threshold(value).is_err(), "Accepted {value}");
        }
    }
    #[test]
    fn gb_matches_original_decimal_n2_both_locales() {
        assert_eq!(gb(1_000_000_000, "en"), "1.00 GB");
        assert_eq!(gb(1_000_000_000, "ru"), "1,00 ГБ");
        assert_eq!(gb(-5_500_000_000, "en"), "-5.50 GB");
        assert_eq!(gb(1_073_741_824, "en"), "1.07 GB");
        assert_eq!(gb(1_005_000_000, "en"), "1.01 GB");
        assert_eq!(gb(-1, "en"), "0.00 GB");
        assert_eq!(gb(i64::MAX, "en"), "9,223,372,036.85 GB");
        assert_eq!(gb(i64::MIN, "ru"), "-9\u{a0}223\u{a0}372\u{a0}036,85 ГБ");
    }
    #[test]
    fn every_original_ru_en_resource_is_preserved() {
        for (language, source) in [
            (
                "ru",
                include_str!("../tests/fixtures/legacy/Strings.ru.xaml"),
            ),
            (
                "en",
                include_str!("../tests/fixtures/legacy/Strings.en.xaml"),
            ),
        ] {
            let mut count = 0;
            for line in source.lines() {
                let Some(tail) = line.split("x:Key=\"").nth(1) else {
                    continue;
                };
                let (key, body) = tail.split_once('\"').unwrap();
                let expected = body
                    .split_once('>')
                    .unwrap()
                    .1
                    .split_once("</sys:String>")
                    .unwrap()
                    .0;
                assert_eq!(text(language, key), expected, "{language}:{key}");
                count += 1;
            }
            assert_eq!(count, 227);
        }
        assert_eq!(text("ru", "Missing.Resource.Key"), "Missing.Resource.Key");
        assert_eq!(text("en", "Action.Scan"), "Scan now");
        assert_eq!(text("ru", "Action.Scan"), "Проверить сейчас");
    }
}
