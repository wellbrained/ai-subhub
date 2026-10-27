using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Hub.Core;
using Xunit;

namespace Hub.App.Tests;

public sealed class SubscriptionUiTests
{
    [Fact]
    public async Task LongAllowanceLabelsWrapAndBillingSurvivesAccountRename()
    {
        if (!OperatingSystem.IsWindows()) return;
        const string isolated = "AISUBHUB_SUBSCRIPTION_UI_TEST";
        if (Environment.GetEnvironmentVariable(isolated) != "1")
        {
            var dotnet = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../.tools/dotnet/dotnet.exe"));
            var start = new System.Diagnostics.ProcessStartInfo(File.Exists(dotnet) ? dotnet : "dotnet")
            {
                RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true
            };
            start.ArgumentList.Add("vstest"); start.ArgumentList.Add(typeof(SubscriptionUiTests).Assembly.Location);
            start.ArgumentList.Add("--TestCaseFilter:FullyQualifiedName=Hub.App.Tests.SubscriptionUiTests.LongAllowanceLabelsWrapAndBillingSurvivesAccountRename");
            start.Environment[isolated] = "1";
            using var process = System.Diagnostics.Process.Start(start)!;
            var output = process.StandardOutput.ReadToEndAsync(); var error = process.StandardError.ReadToEndAsync();
            try { await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(40)); }
            catch { process.Kill(entireProcessTree: true); throw; }
            Assert.True(process.ExitCode == 0, await output + await error);
            return;
        }
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread = new Thread(() =>
        {
            try
            {
                AppBuilder.Configure<App>().UsePlatformDetect().SetupWithoutStarting();
                using var hub = new HubController(true);
                var account = hub.Settings.Accounts[0];
                hub.Results[account.Id] = new(UsageState.Ready, "Connected", new(DateTimeOffset.UtcNow,
                    [new("Weekly · Very long model-specific allowance", 12, null)]));
                var window = new MainWindow(hub); window.Show(); Pump();
                try
                {
                    var label = Assert.Single(window.GetVisualDescendants().OfType<TextBlock>(), text => text.Text == "Weekly · Very long model-specific");
                    Assert.Equal(TextWrapping.Wrap, label.TextWrapping);
                    Assert.Equal(TextTrimming.None, label.TextTrimming);
                    Assert.True(label.Bounds.Height > 20);
                    Assert.True(label.DesiredSize.Width <= label.Bounds.Width + 1);
                    var reset = window.GetVisualDescendants().OfType<TextBlock>().First(text => text.Text == "Reset time unavailable");
                    Assert.Equal(TextWrapping.Wrap, reset.TextWrapping);
                    Assert.Equal(TextTrimming.None, reset.TextTrimming);
                    Assert.True(reset.Bounds.Height > 20);
                    Assert.True(reset.DesiredSize.Width <= reset.Bounds.Width + 1);
                    Assert.Equal(620, window.ClientSize.Width, 0);

                    var editor = Dialogs.AccountWindow(hub, account.Provider, account);
                    var editedResult = editor.ShowDialog<AccountProfile?>(window); Pump();
                    editor.GetVisualDescendants().OfType<TextBox>().First().Text = "Renamed";
                    var save = editor.GetVisualDescendants().OfType<Button>().Single(button => button.Content as string == "Save");
                    save.RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent)); Pump();
                    var edited = editedResult.GetAwaiter().GetResult();
                    Assert.Equal(account.Billing, edited!.Billing);

                    var billing = MonthlyBillingUi.EditorWindow(account); billing.Show(window); Pump();
                    Assert.Equal(account.Billing!.Date.ToDateTime(TimeOnly.MinValue), billing.GetVisualDescendants().OfType<CalendarDatePicker>().Single().SelectedDate);
                    Assert.True(billing.Bounds.Height <= 520); billing.Close();

                    var today = DateOnly.FromDateTime(DateTime.Now);
                    var oldAnchor = today.AddMonths(-2);
                    var recurring = account with { Billing = new(oldAnchor) };
                    hub.Settings.Accounts[0] = recurring;
                    var editing = MonthlyBillingUi.Edit(window, hub, recurring); Pump();
                    var monthlyEditor = window.OwnedWindows.Single(owned => owned.Title == "Monthly billing · AI SubHub");
                    Assert.Equal(BillingSchedule.NextDate(recurring.Billing!, today).ToDateTime(TimeOnly.MinValue), monthlyEditor.GetVisualDescendants().OfType<CalendarDatePicker>().Single().SelectedDate);
                    monthlyEditor.GetVisualDescendants().OfType<Button>().Single(button => button.Content as string == "Save")
                        .RaiseEvent(new Avalonia.Interactivity.RoutedEventArgs(Button.ClickEvent)); Pump();
                    editing.GetAwaiter().GetResult();
                    Assert.Equal(oldAnchor, hub.Settings.Accounts[0].Billing!.Date);

                    var reminder = new BillingReminderWindow(hub);
                    reminder.Add([BillingSchedule.Reminder(account, DateOnly.FromDateTime(DateTime.Now))!]);
                    window.Hide(); reminder.Show(); Pump();
                    Assert.True(reminder.IsVisible); Assert.False(window.IsVisible); reminder.Close();
                }
                finally { window.Close(); Pump(); }
                completion.SetResult();
            }
            catch (Exception ex) { completion.SetException(ex); }
        });
        thread.SetApartmentState(ApartmentState.STA); thread.Start();
        await completion.Task.WaitAsync(TimeSpan.FromSeconds(30));
    }

    private static void Pump()
    {
        using var stop = new CancellationTokenSource();
        DispatcherTimer.RunOnce(stop.Cancel, TimeSpan.FromMilliseconds(150));
        Dispatcher.UIThread.MainLoop(stop.Token);
    }
}
