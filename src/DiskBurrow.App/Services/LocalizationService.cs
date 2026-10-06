using System.Globalization;
using System.Xml.Linq;
using System.Windows;
namespace DiskBurrow.App.Services;
public sealed class LocalizationService
{
    private static readonly Dictionary<string,Dictionary<string,string>> Texts = new() { ["ru"] = Load("ru"),["en"] = Load("en") };
    private ResourceDictionary? active;
    public string Language {get;private set;}="ru";
    public event Action? Changed;
    private static Dictionary<string,string> Load(string language)
    {
        using var stream=typeof(LocalizationService).Assembly.GetManifestResourceStream($"DiskBurrow.App.Resources.Strings.{language}.xaml") ?? throw new InvalidOperationException("Locale resources missing.");
        return XDocument.Load(stream).Root!.Elements().ToDictionary(e=>(string)e.Attribute(XName.Get("Key","http://schemas.microsoft.com/winfx/2006/xaml"))!,e=>e.Value);
    }
    public static string[] ReadKeys(string language)=>Texts[language].Keys.ToArray();
    public static string ReadText(string language,string key)=>Texts[language].GetValueOrDefault(key,Texts[language]["Unknown"]);
    public string Text(string key)=>ReadText(Language,key);
    public void SetLanguage(string language)
    {
        if(!Texts.ContainsKey(language))throw new ArgumentException("Unsupported language.");
        Language=language;CultureInfo.CurrentCulture=CultureInfo.GetCultureInfo(language);CultureInfo.CurrentUICulture=CultureInfo.CurrentCulture;
        if(Application.Current is {} app){if(active is not null)app.Resources.MergedDictionaries.Remove(active);active=new(){Source=new Uri($"Resources/Strings.{language}.xaml",UriKind.Relative)};app.Resources.MergedDictionaries.Add(active);}
        Changed?.Invoke();
    }
}
