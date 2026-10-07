# DiskBurrow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Выбор способа выполнения принадлежит пользователю; до проверки этого плана продуктовый код не создаётся.

**Goal:** Выпустить Windows-приложение в трее, которое объясняет заполнение диска, сохраняет историю роста и удаляет выбранные временные файлы по проверяемым правилам; опубликовать исходники и portable-выпуск на GitHub.

**Architecture:** WPF-приложение координирует независимые интерфейсы сканера, хранилища, планировщика и очистки. Core не зависит от UI, Windows-адаптеры работают с проверенными file handles, Storage отвечает за SQLite и ограниченное хранение. Фоновая работа и ручные операции проходят через одного координатора и отменяемые задачи.

**Tech Stack:** C# / .NET 10, WPF, WinForms NotifyIcon, Microsoft.Data.Sqlite, xUnit, Windows Win32 API, GitHub Actions.

**Spec:** [DiskBurrow-design.md](DiskBurrow-design.md), согласованная спецификация от 2026-10-05. В Task 1 зафиксировано уточнение покрытия: отказ доступа в одном поддереве не мешает сравнению других, полностью проверенных поддеревьев.

**Workspace:** будущий репозиторий `<workspace>/DiskBurrow`. Все пути ниже относительны этому корню. План и спецификация копируются в `docs/superpowers/` в Task 1. Репозиторий и продуктовый код пока не создавались.

## Global Constraints

- Windows 10 22H2 / Windows 11 x64; .NET 10; portable ZIP; self-contained win-x64.
- Публичный GitHub-репозиторий DiskBurrow; лицензия MIT; русский и английский интерфейсы; русский по умолчанию.
- Обычная работа без службы, повышения прав, телеметрии и фоновых сетевых запросов.
- Системный диск определяет Windows; дополнительные локальные диски выбирает пользователь; сетевые диски не сканируются в фоне.
- Полная проверка каждые 6 часов; первая фоновая проверка ждёт 5 минут; доступные интервалы 1, 6, 12 и 24 часа.
- Проверка свободного места каждые 5 минут; предупреждение ниже 15 ГБ; порог роста папки 5 ГБ; повтор по одной папке не чаще одного раза в сутки.
- На батарее полный фоновый обход откладывается по умолчанию; пропущенные проверки объединяются; параллельных проверок нет.
- Автозапуск выключен до явного включения; тестирование не изменяет настоящий HKCU Run пользователя.
- История: последние 30 завершённых проверок, до 100 крупнейших файлов на проверку; общий бюджет базы, служебных файлов SQLite и логов 250 МиБ; логи до 10 МиБ.
- Правила: пользовательский TEMP старше 7 суток; пользовательские CrashDumps/.dmp старше 7 суток; только Chrome/Edge Cache/Cache_Data при закрытом браузере.
- Reparse points не обходятся; облачные placeholders не скачиваются; hard links не удваивают физический объём.
- Очистка постоянная, только после просмотра и явного выбора файлов; изменившиеся, заблокированные и непроверенные файлы пропускаются.
- Реальный C: проверяется только на чтение; destructive-тесты работают исключительно с временными фикстурами.

## Review Focus

1. Подмена родительской папки ссылкой после preview: удаление не должно выйти за разрешённую область — Task 4, `AncestorReplacementCannotDeleteOutsideRuleRoot`.
2. TEMP перенаправлен на профиль, корень тома или сходный префикс: правило не должно предложить пользовательские документы — Task 3, `BroadAndPrefixSiblingRootsAreRejected`.
3. Нет места для SQLite или отказ доступа только в части диска: приложение показывает текущий результат, не выдумывает удалённые папки и продолжает мониторинг — Task 2, `StorageFailurePreservesLiveResult`, `DeniedSubtreeDoesNotBecomeDeletion`.
4. Перезапуск, сон, смена часов и два запуска одновременно: одна проверка и один основной процесс; уведомления не спамят — Task 5, `MissedIntervalsCoalesceAfterResume`, Task 6, `SecondInstanceActivatesFirst`.
5. Экспорт и выпуск содержат персональные пути или локальную историю: отчёт требует отдельного действия, пакет строится только из allowlist — Task 6, `ExportRequiresExplicitAction`, Task 7, `PackageContainsOnlyPublishedFiles`.

## Файлы и общие контракты

- `DiskBurrow.slnx`, `global.json`, `Directory.Build.props`, `.gitignore`: сборка, nullable, lock-файлы зависимостей, исключение истории, отчётов и артефактов.
- `src/DiskBurrow.Core/Scanning/ScanModels.cs`: `FileIdentity(ulong Volume, string FileId)`, `FileObservation(string Path, FileIdentity? Identity, long LogicalBytes, long? AllocatedBytes, DateTimeOffset ModifiedUtc, int LinkCount, FileAttributes Attributes)`, `DirectoryObservation(string Path, long LogicalBytes, long? AllocatedBytes, bool CoverageComplete)`.
- Там же: `ScanSnapshot(Guid Id, string Root, DateTimeOffset StartedUtc, DateTimeOffset CompletedUtc, bool TraversalCompleted, IReadOnlyList<DirectoryObservation> Directories, IReadOnlyList<FileObservation> LargestFiles, IReadOnlyList<ScanIssue> Issues)`, `ScanProgress(long FilesVisited, long DirectoriesVisited, string CurrentPath)`, `ScanIssue(string Path, ScanIssueKind Kind)`.
- Enum ScanIssueKind: AccessDenied, ReparseSkipped, CloudSkipped, MetadataUnavailable, ChangedDuringScan, IoFailure. Enum FolderChangeKind: New, Removed, Changed, Unavailable. Enum CleanupOutcome: Deleted, Missing, SkippedChanged, SkippedBusy, SkippedPolicy, Failed; `CleanupItemResult(Guid CandidateId, CleanupOutcome Outcome, string? ReasonKey)`.
- `src/DiskBurrow.Core/Scanning/IDiskScanner.cs`: `Task<ScanSnapshot> ScanAsync(string root, IProgress<ScanProgress>? progress, CancellationToken cancellationToken)`.
- `src/DiskBurrow.Windows/Files/NativeFileApi.cs`, `WindowsDiskScanner.cs`: native handles, metadata, идентификаторы, выделенный объём, обход.
- `src/DiskBurrow.Core/History/SnapshotComparer.cs`: `IReadOnlyList<FolderChange> Compare(ScanSnapshot previous, ScanSnapshot current)`, где `FolderChange(string Path, long LogicalDeltaBytes, long? AllocatedDeltaBytes, FolderChangeKind Kind, bool Comparable)`.
- `src/DiskBurrow.Core/History/IHistoryStore.cs`: `Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot, CancellationToken ct)`, `Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root, int limit, CancellationToken ct)`, `Task AppendCleanupAsync(CleanupReport report, CancellationToken ct)`; `HistoryWriteResult(bool Saved, string? UserMessage)`.
- `src/DiskBurrow.Storage/SqliteHistoryStore.cs`, `StorageBudget.cs`, `SettingsStore.cs`: история, бюджет и настройки с атомарным сохранением.
- `src/DiskBurrow.Core/Cleanup/CleanupModels.cs`: `RuleRoot(string RuleId, string Path, TimeSpan? MinimumAge, string? OwnerProcessName)`, `CleanupCandidate(Guid Id, RuleRoot Rule, FileObservation File, string ReasonKey)`, `CleanupPlan(Guid Id, DateTimeOffset CreatedUtc, IReadOnlyList<CleanupCandidate> Candidates)`, `CleanupReport(Guid PlanId, IReadOnlyList<CleanupItemResult> Items, long FreeSpaceDeltaBytes)`; результаты используют enum, а не локализованные строки.
- `src/DiskBurrow.Core/Cleanup/ICleanupPlanner.cs`: `Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excludedPaths, CancellationToken ct)`; `ICleanupExecutor.cs`: `Task<CleanupReport> ExecuteAsync(CleanupPlan plan, IReadOnlySet<Guid> selectedIds, CancellationToken ct)`.
- `src/DiskBurrow.Windows/Cleanup/RuleDiscovery.cs`, `WindowsCleanupPlanner.cs`, `WindowsCleanupExecutor.cs`, `AncestorLease.cs`: ограниченные корни, preview и удаление с удержанием проверенных handles.
- `src/DiskBurrow.Core/Monitoring/AppSettings.cs`, `SchedulePolicy.cs`, `AlertPolicy.cs`, `MonitoringCoordinator.cs`, `IMonitoringEnvironment.cs`: значения по умолчанию, расписание, состояние подавления уведомлений, координация через платформенный интерфейс.
- `src/DiskBurrow.Windows/System/WindowsEnvironment.cs`, `AutostartRegistration.cs`: системный том, питание, свободное место и отдельный адаптер реестра.
- `src/DiskBurrow.App/App.xaml`, `MainWindow.xaml`, `ViewModels/`, `Services/`, `Resources/`: UI, DI-композиция вручную, трей, координация, single instance, локализация, отчёты, рекомендации и собственная иконка.
- `tests/DiskBurrow.Tests/`: Core и Windows-интеграция на временных каталогах; `work/` в репозитории игнорируется.
- `scripts/publish.ps1`, `.github/workflows/ci.yml`, `README.md`, `README.ru.md`, `LICENSE`, `docs/verification.md`: воспроизводимый выпуск и доказательства проверки.

## Task 1: Сканирование и корректный учёт места

**Files:** создаются решение, Core/Scanning, Windows/Files, `tests/DiskBurrow.Tests/ScanningTests.cs`, `NativeMetadataTests.cs`, `TempTree.cs`, инфраструктура сборки и копии плана/спецификации в `docs/superpowers/`.

**Interfaces:** производит `IDiskScanner`, `ScanSnapshot` и перечисленные модели. NativeFileApi предоставляет `FileObservation Inspect(string path)` и `SafeFileHandle OpenMetadata(string path)`; scanner использует только чтение атрибутов, не содержимого файлов.

- [ ] Создать локальный Git-репозиторий и решение, `net10.0` для Core, `net10.0-windows` для Windows и Tests. Закрепить доступный .NET SDK 10.0.100 с patch-roll-forward. Зафиксировать точные совместимые версии тестовых пакетов из официальной NuGet metadata и lock-файлы; не добавлять GUI до Task 6.
- [ ] Написать failing-тесты: `TwoHardLinksCountOnePhysicalAllocation` (два имени, один идентификатор, физический итог один), `JunctionCycleIsSkipped`, `CloudPlaceholderDoesNotReadContent`, `DeniedSubtreeMarksAncestorsIncomplete` (другой доступный sibling остаётся полным), `CancellationDoesNotProduceCompletedSnapshot`, `EmptyAndLongPathTreesScan`, `ChangedFileIsApproximate`. Фикстуры автоматически проверяют, что их абсолютные пути находятся под собственным временным корнем.
  Assertions для hard-link fixture: `Assert.Equal(first.Identity, second.Identity); Assert.Equal(first.AllocatedBytes, snapshot.Directories.Single(d => d.Path == root).AllocatedBytes);`. Для отмены: `await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scanner.ScanAsync(root, null, cancelledToken));`.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~ScanningTests|FullyQualifiedName~NativeMetadataTests"`: подтвердить ожидаемый FAIL до реализации, а не failure восстановления пакетов.
- [ ] Реализовать `ScanAsync`: один обход; максимум 4 одновременных metadata-handles; UI-progress не чаще 4 раз в секунду; отмена между операциями. Проверять атрибуты до открытия облачной записи. Использовать FileIdInfo, FileStandardInfo и FileBasicInfo через handles; при недоступности физического размера возвращать null и issue. Сам файл не читать. Для hard links выбрать одну детерминированную каноническую запись и отнести физические байты только к её ancestors; агрегацию выполнить после выбора канонических путей. Хранить только top 100 файлов и folder aggregates. Не складывать неизвестный allocated size как ноль.
- [ ] Завершённый обход может иметь incomplete coverage отдельных поддеревьев. Отмена возвращает OperationCanceledException; частичный результат отображается как прогресс, но не сохраняется как завершённая история. DriveInfo отдельно даёт свободное место и общий занятый объём.
- [ ] Повторить целевые тесты: PASS. При отсутствии прав на symlink-test записать конкретный SKIP; hard-link и junction-проверки обязательны на NTFS. Не включать Developer Mode и не менять ACL реальных пользовательских каталогов.
- [ ] Commit: `feat: scan disk metadata with explicit coverage and link accounting` — только файлы этой задачи и docs.

## Task 2: История, сравнение и ограниченное хранение

**Files:** Core/History, Storage, `tests/DiskBurrow.Tests/HistoryTests.cs`, `StorageBudgetTests.cs`.

**Interfaces:** потребляет Task 1; производит `SnapshotComparer`, `IHistoryStore`, `HistoryWriteResult`, `FolderChange`. `SettingsStore.LoadAsync(CancellationToken ct)` / `SaveAsync(AppSettings settings, CancellationToken ct)` добавляются в Task 5 после определения AppSettings.

- [ ] Написать failing-тесты: `GrowthAndShrinkAreCompared` (10→15 ГиБ даёт +5), `DeniedSubtreeDoesNotBecomeDeletion`, `CoveredSiblingCanBeCompared` (denied Windows subtree не отключает полностью покрытый cache subtree), `DisappearedFolderRequiresCoveredParent`, `OnlyCompletedTraversalsAreStored`, `ThirtySnapshotsAndHundredFilesAreRetained`, `StorageFailurePreservesLiveResult`, `CorruptHistoryDoesNotCrashMonitoring`, `SchemaMigrationPreservesSnapshots`.
  Assertions: `Assert.Equal(5L * 1024 * 1024 * 1024, changes.Single().LogicalDeltaBytes); Assert.Equal(30, retained.Count); Assert.All(retained, s => Assert.InRange(s.LargestFiles.Count, 0, 100)); Assert.False(failedWrite.Saved);`.
- [ ] Написать budget-тесты через настраиваемый малый лимит: `BudgetIncludesDatabaseSidecarsAndLogs`, `OversizeSnapshotIsRejectedWithoutUnboundedGrowth`, `TenMiBLogsRotate`, `RestartKeepsPreviousCompleteHistory`. В production values проверить ровно 250 * 1024 * 1024 и 10 * 1024 * 1024 байт.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~HistoryTests|FullyQualifiedName~StorageBudgetTests"`: ожидаемый FAIL.
- [ ] Реализовать comparer по нормализованным путям Windows с ordinal-ignore-case. Рост уведомляется по LogicalDeltaBytes; UI явно отличает его от известного физического вклада, особенно для hard links. Не сравнивать неполные поддеревья и не считать пропущенные папки удалёнными.
- [ ] Реализовать SQLite со schema-version, параметризованными запросами и сериализацией writers. Хранить агрегаты и issues, а не содержимое. Выбрать journal mode DELETE, чтобы не держать постоянный WAL; всё равно учитывать любые sidecars и временные файлы. До записи оценивать её бюджет, удалять старые записи, ограничивать max_page_count с резервом для journal, использовать incremental vacuum и ограниченный журнал. Не выполнять VACUUM с полной копией базы на почти полном диске. Если запись не укладывается — возвращать Saved=false и понятную причину, сохраняя live result в памяти.
- [ ] Сохранять миграции транзакционно. При повреждении базы не удалять её молча: предлагать явный сброс истории; live scan доступен. Сохранение отказавшего снимка не меняет last successful snapshot и last completed traversal в памяти.
- [ ] Повторить тесты: PASS; закрыть все соединения и проверить reopen. Commit: `feat: keep bounded local history and compare covered folders`.

## Task 3: Каталог очистки и предварительный просмотр

**Files:** Core/Cleanup models и planner interface, Windows/Cleanup rule discovery/planner, `tests/DiskBurrow.Tests/CleanupPreviewTests.cs`, `RuleDiscoveryTests.cs`.

**Interfaces:** потребляет FileObservation и NativeFileApi; производит `ICleanupPlanner`, `CleanupPlan`, `RuleRoot`. `RuleDiscovery.Discover()` возвращает известные корни, предупреждения и owner process, но не позволяет строке из UI создавать произвольное правило.

- [ ] Написать failing-тесты: `OnlyFilesOlderThanSevenDaysAreProposed` (строго старше, ровно 7 дней пропустить), `BroadAndPrefixSiblingRootsAreRejected`, `JunctionAncestorsAreRejected`, `CrashDumpRuleRequiresDmpExtension`, `BrowserRuleKeepsCookiesHistoryAndPasswords`, `RunningBrowserDisablesItsRule`, `ExclusionsApplyToDescendants`, `PreviewNeverMutatesFilesystem`, `NoCandidateIsInitiallySelected`.
  Assertions: `Assert.DoesNotContain(plan.Candidates, c => c.File.Path == exactlySevenDaysPath); Assert.DoesNotContain(plan.Candidates, c => c.File.Path == cookiesPath); Assert.Equal(beforeTreeHash, afterTreeHash);`.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~CleanupPreviewTests|FullyQualifiedName~RuleDiscoveryTests"`: ожидаемый FAIL.
- [ ] Реализовать каталог: системный GetTempPath рассматривается только как hint. Автоматически разрешать стандартный LocalAppData/Temp; нестандартный TEMP требует явного сохранения узкого разрешённого корня после проверки, а широкие области, профиль, Windows, Program Files, ProgramData и корни томов отклонять всегда. Такие разрешения являются настройками rules, не универсальным механизмом удаления путей.
- [ ] Для Chrome/Edge обнаруживать только Default и Profile * внутри стандартного User Data. Каждый cache root — точный Cache/Cache_Data; root и ancestors не reparse. Проверять процесс браузера до preview и возвращать unavailable при невозможности проверки. Для dump-root разрешать только текущий LocalAppData/CrashDumps. Игнорировать директории, read-only, reparse/облачные файлы и alternate data stream paths. Возраст относится к ModifiedUtc относительно одного captured UTC instant.
- [ ] Реализовать `PreviewAsync`: индивидуальные кандидаты с ID, identity, bytes, mtime и ReasonKey; нормализованные исключения применяются по границе path-segment. Preview показывает сумму размеров данных как оценку кандидатов, не обещает физическое освобождение.
- [ ] Повторить тесты: PASS; verify bytes/tree unchanged до и после preview. Commit: `feat: preview narrowly scoped cleanup rules`.

## Task 4: Выполнение очистки без выхода за разрешённые пути

**Files:** ICleanupExecutor, Windows/Cleanup executor и AncestorLease, дополняется NativeFileApi, `tests/DiskBurrow.Tests/CleanupExecutionTests.cs`, `CleanupRaceTests.cs`.

**Interfaces:** потребляет CleanupPlan и selected IDs; производит CleanupReport. `AncestorLease.OpenVerified(RuleRoot root, string candidatePath)` удерживает проверенные directory handles; `NativeFileApi.MarkForDeletion(SafeFileHandle file)` работает только с тем file handle, который проверил executor.

- [ ] Написать failing-тесты: `OnlySelectedCandidatesAreDeleted`, `ChangedIdentitySizeOrMtimeIsSkipped`, `LockedAndReadonlyFilesAreSkipped`, `BrowserReopenedAfterPreviewIsSkipped`, `AncestorReplacementCannotDeleteOutsideRuleRoot`, `FileReplacedBySymlinkIsSkipped`, `UnknownSelectedIdIsRejected`, `RootAndDirectoriesAreNeverDeleted`, `CancellationStopsFurtherDeletions`, `DeletionReportSeparatesMissingSkippedFailedAndDeleted`.
  Assertions для race fixture: `Assert.True(File.Exists(outsideSentinelPath)); Assert.All(report.Items, item => Assert.NotEqual(CleanupOutcome.Deleted, item.Outcome));`; для selection fixture: `Assert.False(File.Exists(selectedPath)); Assert.True(File.Exists(unselectedPath)); Assert.True(Directory.Exists(ruleRoot));`.
- [ ] Тесты подмены используют изолированные in-root/outside-rule фикстуры под одним тестовым корнем, синхронизационный hook перед открытием target и отдельный hook после lease. Assert: вне rule не удалён ни один файл. Не использовать настоящий TEMP пользователя как rule-root даже в интеграционном тесте.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~CleanupExecutionTests|FullyQualifiedName~CleanupRaceTests"`: ожидаемый FAIL.
- [ ] Реализовать revalidation: rule заново обнаружен и соответствует сохранённому разрешению; selected ID принадлежит плану; путь обычный абсолютный local path, без ADS/device namespace. Последовательно открыть и удерживать все ancestors от корня тома до родителя файла с OPEN_EXISTING, BACKUP_SEMANTICS и OPEN_REPARSE_POINT, запретив sharing на изменение/удаление там, где необходимо для защиты от подмены. По handle проверить атрибуты, identity и конечный путь. При конфликте sharing пропускать файл.
- [ ] Открыть target с DELETE и FILE_READ_ATTRIBUTES, без разрешения параллельной записи/замены, с OPEN_REPARSE_POINT. Повторно сравнить identity, size, mtime, age и owner process. Использовать SetFileInformationByHandle(FileDispositionInfo) и затем закрыть этот handle; никаких DeleteFile(path) после проверки и рекурсивных Remove-Item. Ссылки и read-only пропускать; права и атрибуты не менять. Документация API приведена в конце плана.
- [ ] Отчёт хранит outcomes и observed free-space delta после закрытия handles. Если append журнала не удался, показать итог операции и отдельно ошибку истории, не повторять удаление.
- [ ] Повторить тесты: PASS; stress-подмена ancestors не удаляет outside-rule sentinel. Commit: `feat: execute reviewed cleanup through verified file handles`.

## Task 5: Расписание, предупреждения и настройки

**Files:** Core/Monitoring, Storage/SettingsStore, Windows/System, `tests/DiskBurrow.Tests/MonitoringTests.cs`, `SettingsTests.cs`.

**Interfaces:** потребляет scanner/history/comparer. Производит `AppSettings` с interval, delay, thresholds, language, excluded paths, paused flag, battery permission и autostart choice. `IMonitoringEnvironment` предоставляет `string SystemRoot`, `bool IsOnBattery` и `long GetFreeBytes(string root)`; WindowsEnvironment реализует его, Core не ссылается на Windows project. `SchedulePolicy.IsFullScanDue(DateTimeOffset nowUtc, bool onBattery, bool operationRunning)`; `AlertPolicy.Evaluate(ScanSnapshot? previous, ScanSnapshot? current, long freeBytes, DateTimeOffset nowUtc)` возвращает `IReadOnlyList<AlertEvent>` с destination path. `MonitoringCoordinator.RequestScanAsync(CancellationToken ct)`, `CancelScan()`, `Pause(bool paused)`; coordinator хранит live result даже при отказе history. `AlertSuppressionState` содержит low-space latch и UTC last-notified по нормализованному root/path; SettingsStore добавляет `LoadAlertStateAsync(CancellationToken ct)` / `SaveAlertStateAsync(AlertSuppressionState state, CancellationToken ct)`.

- [ ] Написать failing-тесты с fake clock/environment: `DefaultsMatchSpec`, `LowSpaceAlertsOnceUntilRecovery`, `FiveGBGrowthAlertsOnCoveredFolder`, `GrowthDeduplicatesForTwentyFourHours`, `ConcurrentRequestsShareOneScan`, `BatteryDefersBackgroundButAllowsManual`, `MissedIntervalsCoalesceAfterResume`, `ClockMovedBackDoesNotSpam`, `PausedMonitoringDoesNotStartFullScan`, `ExitCancelsAndDoesNotSavePartialScan`, `StorageFailureDoesNotDisableFreeSpaceCheck`.
  Assertions: `Assert.Single(firstLowSpaceAlerts); Assert.Empty(repeatedLowSpaceAlerts); Assert.Equal(1, fakeScanner.InvocationCount); Assert.Empty(growthAlertsTwentyThreeHoursLater);`. Проверять повторный alert через полные 24 часа только для нового значимого роста, а не воспроизведения старого снимка.
- [ ] Settings-тесты: `InvalidIntervalAndNegativeThresholdsAreRejected`, `SettingsSaveIsAtomic`, `InvalidSettingsRecoverToVisibleDefaults`, `AutostartWritesOnlyDiskBurrowValue`, `MovedExeIsDetected`, `AutostartOffDoesNotTouchRegistry`. Registry tests используют in-memory adapter, а не настоящий HKCU Run.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~MonitoringTests|FullyQualifiedName~SettingsTests"`: ожидаемый FAIL.
- [ ] Реализовать default values из Global Constraints; thresholds и intervals валидировать на загрузке и при сохранении. Сохранять состояние suppression между рестартами. Планирование на одном loop, монотонный таймер для ожиданий, UTC для снимков; resume объединяет пропуски. Pause останавливает полные обходы, а low-space check продолжает работать и это объяснено в UI.
- [ ] Environment определяет системный том через Windows directory; Windows power status и DriveInfo доступны без повышения прав. HKCU Run adapter пишет quoted absolute EXE path, удаляет только собственную запись с соответствующим payload. Изменения выполняются только из пользовательского Save Settings. Фоновый процесс не вызывает очистку.
- [ ] Повторить тесты: PASS. Commit: `feat: monitor disk growth with persistent settings and quiet alerts`.

## Task 6: Приложение, трей и пользовательские действия

**Files:** App WPF project/manifest, MainWindow, ViewModels/OverviewViewModel.cs, HistoryViewModel.cs, CleanupViewModel.cs, SettingsViewModel.cs; Services/TrayService.cs, SingleInstanceService.cs, LocalizationService.cs, ReportExporter.cs, Recommendations.cs; Resources/Strings.ru.xaml, Strings.en.xaml, собственный icon; `tests/DiskBurrow.Tests/PresentationTests.cs`, `SingleInstanceTests.cs`.

**Interfaces:** потребляет Task 1–5. `CleanupViewModel.AnalyzeAsync(CancellationToken ct)`, `ExecuteSelectedAsync(bool permanentDeletionConfirmed, CancellationToken ct)`, `bool CanExecuteCleanup`, `IReadOnlySet<Guid> SelectedIds`; `ReportExporter.ExportAsync(ScanSnapshot snapshot, string destination, bool pathsDisclosureAccepted, CancellationToken ct)`; `SingleInstanceService.TryBecomePrimaryAsync()` и activation IPC с доступом только текущему пользователю.

- [ ] Написать failing-тесты моделей представления: `NoCandidateIsInitiallySelected`, `ExecuteRequiresSelectionAndPermanentDeletionConfirmation`, `SelectionSurvivesFiltering`, `UiReportsCoverageAndUnknownAllocatedBytes`, `HistoryFailureLeavesCurrentOverviewVisible`, `ExportRequiresExplicitAction`, `RussianAndEnglishResourceKeysMatch`, `AllActionsHaveLocalizedStatus`, `SecondInstanceActivatesFirst`.
  Assertions: `Assert.Empty(viewModel.SelectedIds); Assert.False(viewModel.CanExecuteCleanup); Assert.Equal(0, fakeExecutor.InvocationCount); Assert.False(File.Exists(unconfirmedExportPath)); Assert.Equal(russianKeys.Order(), englishKeys.Order());`.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~PresentationTests|FullyQualifiedName~SingleInstanceTests"`: ожидаемый FAIL.
- [ ] Создать App с ручной DI-композицией. Основная навигация: обзор, крупные папки/файлы, история/изменения, очистка, настройки. Использовать виртуализированные списки и выбор диска/папки; все долгие действия async, status и cancel доступны. Показывать дату проверки, известные/неизвестные размеры, coverage и отличия суммы от Windows used space. UI-observable updates только через Dispatcher; exceptions не теряются в async void.
- [ ] Cleanup screen содержит категории и файлы, причины, суммы, исключения, отдельное предупреждение о необратимости и confirmation. Повторное нажатие не запускает второй executor. После выполнения показать individual outcomes и free-space delta. Системные действия открывают штатные Storage settings; рекомендации других кэшей открывают ссылки только по нажатию, не запускают неподтверждённые команды и не создают junction.
- [ ] NotifyIcon: открыть, проверить сейчас, пауза, выход; close скрывает окно, Exit/Windows shutdown отменяет операции, disposes tray/IPC/storage и не сохраняет partial scan. Mutex с привязкой к текущему пользователю, именованный pipe с CurrentUserOnly активирует первое окно. Balloon click открывает путь/раздел уведомления, pending alert сохраняется до активации.
- [ ] Settings: interval, thresholds, battery, язык, исключения и явный autostart. Смена языка обновляет видимый интерфейс. Report export спрашивает пользовательский destination и раскрытие путей; не экспортирует автоматически. Ссылки GitHub/Releases устанавливаются после определения реального owner в Task 7.
- [ ] Повторить тесты: PASS. Запустить фактически собранный EXE и проверить открытие, tray, скрытие/возврат, повторный запуск, обе локализации, уведомления, отмену и настройки. Проверки GUI, недоступные автоматизации, перечислить как unverified в docs/verification.md; если нужно — дать пользователю короткий ручной сценарий. Автозапуск и реальные cleanup-кнопки не включать при smoke run.
- [ ] Commit: `feat: ship localized tray interface for disk monitoring and cleanup`.

## Task 7: Реальный выпуск и GitHub

**Files:** scripts/publish.ps1, .github/workflows/ci.yml, README.md, README.ru.md, LICENSE, docs/verification.md; `tests/DiskBurrow.Tests/PackageTests.cs` и GitHub links в App.

**Interfaces:** потребляет готовое App. `scripts/publish.ps1 -Version 0.1.0 -OutputDirectory <path>` собирает `DiskBurrow-0.1.0-win-x64.zip` и `.sha256`; version передаётся как данные, не как исполняемый fragment. Output path проверяется перед рекурсивными удалениями.

- [ ] Написать failing-тесты allowlist: `PackageContainsOnlyPublishedFiles`, `PackageExcludesLocalHistoryReportsAndTestTrees`, `ChecksumMatchesArchive`, `ReadmeDescribesActualRules`. Скрипт создаёт новый output directory и не удаляет чужую папку для удобства.
  Assertions: `Assert.DoesNotContain(entries, e => e.EndsWith(".db", StringComparison.OrdinalIgnoreCase)); Assert.DoesNotContain(entries, e => e.Contains("scan-report", StringComparison.OrdinalIgnoreCase)); Assert.Equal(expectedSha256, actualSha256);`; полный список entries дополнительно сверяется с publish manifest плюс LICENSE/инструкцией.
- [ ] `dotnet test tests/DiskBurrow.Tests --filter "FullyQualifiedName~PackageTests"`: ожидаемый FAIL.
- [ ] Реализовать publish Release / self-contained / win-x64, ZIP из списка publish outputs плюс LICENSE и короткой инструкции. `IncludeNativeLibrariesForSelfExtract` включать только если выбран single-file; обычная папка в ZIP допустима и упрощает SQLite deployment. Не включать базу/логи/отчёты/пользовательские пути/объекты тестов. Собственную иконку и UI resources проверить в final package. Сборка неподписанная, в README честно указано.
- [ ] CI: Windows runner, checkout, setup-dotnet 10.0.x, locked restore, Release build, тесты, publish artifact с checksum. Workflow не выводит секреты и не выполняет произвольные команды из имени версии. Для release-tag создаёт release assets с минимальными permissions; обычный branch run не публикует release.
- [ ] Проверить `dotnet build -c Release`, `dotnet test -c Release`, запуск publish script и PASS package-tests. Развернуть ZIP в независимый `work/portable-smoke` и запустить с изолированными DOTNET_ROOT и DOTNET_MULTILEVEL_LOOKUP=0. Эти переменные сами по себе не доказывают self-contained: дополнительно проверить bundled coreclr/hostpolicy, runtimeconfig и загруженные process modules — CoreCLR должен загружаться из распакованной сборки. Проверить загрузку native SQLite и создание пользовательских данных вне EXE directory.
- [ ] Выполнить read-only scan реального системного диска; записать дату, длительность, coverage, Windows free/used space и поведение отмены. Не публиковать фактические пользовательские paths из этой проверки; docs содержат агрегированное обезличенное свидетельство. Не называть непроверенные GUI-сценарии успешными.
- [ ] Заполнить README на двух языках: установка ZIP, tray/Exit, правила и возраст файлов, необратимость, историю/бюджет, исключения, ограничения hard links/coverage, сборку, tests и manual checks. MIT LICENSE содержит проверенное имя GitHub-владельца, не догадку о его настоящем имени.
- [ ] Проверить доступную GitHub-авторизацию через connector/CLI без вывода токенов. Определить current account, проверить доступность имени DiskBurrow. Если входа нет — попросить пользователя войти; имя занято — запросить новый slug. Не устанавливать gh и не сохранять пароль/токен без отдельной необходимости. До устранения этого ограничения все локальные build/test/docs задачи завершаются.
- [ ] Создать публичный репозиторий в подтверждённом аккаунте, push исходников и tagged 0.1.0, опубликовать первый Release с ZIP/checksum. Авторизация на публикацию уже дана в разговоре; повторно её не спрашивать. Проверить реальный repository URL, результаты CI, release assets и доступность скачивания. Никаких PR не нужно для первичной публикации; если PR всё же создаётся, прикрепить его инструментом Codex.
- [ ] Commit: `build: package portable release and automate Windows validation`; сохранить итоговый tag и ссылку релиза. Финальный отчёт: фактические ссылки, локальный ZIP, результаты тестов и material unverified checks.

## Зависимости и способ выполнения

Задачи выполняются в порядке 1 → 2 → 3 → 4 → 5 → 6 → 7. Task 4 нельзя выпускать без passing race/containment tests; Task 7 не начинается публикацией до проверки готового package.

Рекомендация: **с субагентами**, с независимой проверкой каждой задачи и общей проверкой перед выпуском. У сканера, preview и executor разные обязанности; ошибка их взаимодействия может удалить файлы, поэтому отдельное ревью оправдано. Продуктовые задачи последовательны; параллельное редактирование одних файлов не используется.

Альтернатива: **в этой сессии** основным агентом через executing-plans, затем один отдельный reviewer всего изменения. Это сокращает число передач контекста, но ошибки контрактов могут обнаружиться позже. Выполнение и subagents начинаются только после выбора пользователя.

## Самопроверка плана

- Все требования спецификации распределены: сканирование/coverage → 1; история/лимиты → 2; каталог/preview → 3; удаление/гонки → 4; расписание/настройки → 5; tray/UI/языки/экспорт/рекомендации → 6; runtime/package/GitHub → 7.
- Контракты всех межзадачных типов определены выше; нет зависимости от локализованного текста в domain results.
- Пять Review Focus имеют named tests в своих задачах; версия платформы, defaults, возраст и storage limits заданы точно.
- Полные product method bodies не записаны в план; проверочные шаги отделены от реализации.
- Спецификация уточнена в части покрытия; это явно вынесено для проверки вместе с планом. Остальной согласованный scope сохранён.
- Нельзя считать приложение завершённым только по зелёному build: portable runtime, native SQLite, tray и реальный read-only scan проверяются отдельно.

## Источники для реализации Windows-адаптеров

- [SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle): удаление по проверенному handle и требование DELETE access.
- [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew): OPEN_REPARSE_POINT, directory handles и share modes.
- [GetFileInformationByHandleEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfileinformationbyhandleex): metadata classes; при недоступности страницы использовать официальный header/SDK, не угадывать layouts.

Точные P/Invoke layouts и API-флаги сверяются с официальным SDK при реализации и подтверждаются Windows-интеграционными тестами.
