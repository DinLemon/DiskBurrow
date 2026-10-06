using System.Globalization;

namespace DiskBurrow.App.Services;

public static class ByteDisplay
{
    public const long BytesPerGB = 1_000_000_000;

    public static string Format(long bytes, CultureInfo culture) =>
        $"{(bytes / (decimal)BytesPerGB).ToString("N2", culture)} {(culture.TwoLetterISOLanguageName == "ru" ? "ГБ" : "GB")}";
}
