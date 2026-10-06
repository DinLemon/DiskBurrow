using System.Globalization;
using System.Windows;
using System.Windows.Data;
namespace DiskBurrow.App.Services;
public sealed class ByteConverter : IValueConverter
{
    public object Convert(object value,Type targetType,object parameter,CultureInfo culture)=>value is long number?$"{(number/1073741824d).ToString("N2",culture)} {(culture.TwoLetterISOLanguageName=="ru"?"ГиБ":"GiB")}":LocalizationService.ReadText(culture.TwoLetterISOLanguageName=="ru"?"ru":"en","Unknown");
    public object ConvertBack(object value,Type targetType,object parameter,CultureInfo culture)=>throw new NotSupportedException();
}
public sealed class LocalizedValueConverter : IMultiValueConverter
{
    public object Convert(object[] values,Type targetType,object parameter,CultureInfo culture)
    {
        var lang=values.ElementAtOrDefault(1) as string??"ru";
        var key=values.FirstOrDefault()?.ToString()??"Unknown";
        if(values.FirstOrDefault() is bool b)key=b?"Yes":"No";
        if(parameter is string prefix && prefix.Length>0)key=prefix+key;
        return LocalizationService.ReadText(lang,key);
    }
    public object[] ConvertBack(object value,Type[] targetTypes,object parameter,CultureInfo culture)=>throw new NotSupportedException();
}
public sealed class AllTrueConverter : IMultiValueConverter
{
    public object Convert(object[] values,Type targetType,object parameter,CultureInfo culture)=>values.All(v=>v is true);
    public object[] ConvertBack(object value,Type[] targetTypes,object parameter,CultureInfo culture)=>throw new NotSupportedException();
}
