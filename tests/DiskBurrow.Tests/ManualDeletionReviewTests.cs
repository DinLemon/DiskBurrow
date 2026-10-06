using System.Windows;
using System.Windows.Controls;
using DiskBurrow.App.Services;
using DiskBurrow.App.Views;
using DiskBurrow.Core.Cleanup;

namespace DiskBurrow.Tests;

public class ManualDeletionReviewTests
{
    [Fact] public async Task ReviewShowsExactRootsAndOriginalUnknownReasonInEnglish()
    {
        await Sta(()=>
        {
            var locale=new LocalizationService();locale.SetLanguage("en");
            var plan=new ManualDeletePlan(Guid.NewGuid(),DateTimeOffset.UtcNow,[@"E:\fixture\parent"],[],[new(@"E:\fixture\parent\child","Manual.FutureReason")]);
            var window=new ManualDeletionReviewWindow(plan,null,locale,_=>"0 GiB");
            try
            {
                Assert.Equal("Review manual deletion",window.Title);
                Assert.Contains(Descendants(window).OfType<TextBox>(),box=>box.IsReadOnly&&box.Text==@"E:\fixture\parent");
                Assert.Contains(Descendants(window).OfType<TextBlock>(),block=>block.Text.Contains("Manual.FutureReason"));
                Assert.Contains(Descendants(window).OfType<TextBlock>(),block=>block.Text.Contains("No Recycle Bin or undo"));
                Assert.DoesNotContain(Descendants(window).OfType<Button>(),button=>!button.IsCancel);
            }
            finally{window.Close();}
        });
    }
    [Fact] public void EmittedManualReasonsAndActionsExistInBothLanguages()
    {
        foreach(var language in new[]{"ru","en"})foreach(var key in new[]{"Manual.ProtectedPath","Manual.ConfirmationRequired","Manual.PlanUnavailable","Manual.Changed","Manual.Cancelled","Manual.UnsafePath","Manual.NotEmpty","Manual.Unavailable","Manual.Review","Manual.PermanentWarning","Manual.Analyze","Manual.Delete","Manual.Selected","Manual.Stale"})
            Assert.Contains(key,LocalizationService.ReadKeys(language));
    }
    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        yield return root;
        foreach(var child in LogicalTreeHelper.GetChildren(root).OfType<DependencyObject>())foreach(var descendant in Descendants(child))yield return descendant;
    }
    private static Task Sta(Action action)
    {
        var done=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>{try{action();done.TrySetResult();}catch(Exception error){done.TrySetException(error);}}){IsBackground=true};thread.SetApartmentState(ApartmentState.STA);thread.Start();return done.Task.WaitAsync(TimeSpan.FromSeconds(20));
    }
}
