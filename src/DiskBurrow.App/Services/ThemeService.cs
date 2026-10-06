using System.Windows;

namespace DiskBurrow.App.Services;

public static class ThemeService
{
    private static ResourceDictionary? active;
    public static void Apply(string theme)
    {
        if (theme is not ("light" or "dark")) throw new ArgumentException("Unsupported theme.");
        if (Application.Current is not { } app) return;
        app.Dispatcher.VerifyAccess();
        var next = new ResourceDictionary { Source = new Uri($"/DiskBurrow;component/Resources/Theme.{theme}.xaml", UriKind.Relative) };
        if (active is not null) app.Resources.MergedDictionaries.Remove(active);
        app.Resources.MergedDictionaries.Add(next);
        active = next;
    }
}
