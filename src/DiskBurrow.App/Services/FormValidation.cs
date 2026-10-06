using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Media3D;
namespace DiskBurrow.App.Services;
public static class FormValidation
{
    public static bool HasErrors(DependencyObject root)
    {
        if(Validation.GetHasError(root))return true;
        if(root is not Visual && root is not Visual3D)return false;
        for(var i=0;i<VisualTreeHelper.GetChildrenCount(root);i++)if(HasErrors(VisualTreeHelper.GetChild(root,i)))return true;
        return false;
    }
}
