using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Hub.Core;

namespace Hub.App;

internal static class MonthlyBillingUi
{
    internal const string BillingHint = "Opens your browser's signed-in account, which may differ from this CLI profile. In ChatGPT: Settings → Account → Manage subscription. Apple/Google purchases are managed in that store; team plans may need an admin.";

    internal static Control Footer(Window owner, HubController hub, AccountProfile account)
    {
        var row = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 8, Margin = new Thickness(0, 3, 0, 0) };
        var today = DateOnly.FromDateTime(DateTime.Now);
        var summary = account.Billing is { } billing ? BillingSchedule.Description(billing, today) : "Set monthly renewal";
        var edit = Ui.AsyncButton(summary, () => Edit(owner, hub, account));
        edit.Content = Ui.Text(summary, 11, true); edit.HorizontalAlignment = HorizontalAlignment.Stretch;
        edit.HorizontalContentAlignment = HorizontalAlignment.Left; edit.Padding = new Thickness(0); edit.BorderThickness = new Thickness(0);
        ToolTip.SetTip(edit, "Edit the date from your provider's billing page. Manual dates are not verified; later monthly renewals are estimates. Dates use your local calendar.");
        if (account.Billing is { } configured && BillingSchedule.NextDate(configured, today).DayNumber - today.DayNumber is >= 0 and <= 7)
            ((TextBlock)edit.Content).Foreground = Ui.Accent;
        row.Children.Add(edit);
        var open = Ui.AsyncButton("Open billing", () => OpenBilling(owner, hub, account));
        open.FontSize = 11; open.Padding = new Thickness(6, 2); open.MinHeight = 22; ToolTip.SetTip(open, BillingHint);
        Grid.SetColumn(open, 1); row.Children.Add(open);
        return row;
    }

    internal static async Task OpenBilling(Window owner, HubController hub, AccountProfile account)
    {
        try
        {
            if (await owner.Launcher.LaunchUriAsync(new Uri(BillingSchedule.BillingUrl(account.Provider)))) return;
        }
        catch (Exception ex) when (ex is InvalidOperationException or IOException or System.ComponentModel.Win32Exception) { }
        hub.ReportNotice("Could not open billing. Open the provider's Settings → Billing / Account in your browser.");
    }

    internal static async Task Edit(Window owner, HubController hub, AccountProfile account)
    {
        var current = hub.Settings.Accounts.FirstOrDefault(profile => profile.Id == account.Id);
        if (current is null) return;
        var dialog = EditorWindow(current);
        if (await dialog.ShowDialog<MonthlyBillingEdit?>(owner) is not { } edit) return;
        var index = hub.Settings.Accounts.FindIndex(profile => profile.Id == current.Id);
        if (index < 0) return;
        var previous = hub.Settings.Accounts[index];
        hub.Settings.Accounts[index] = previous with { Billing = edit.Billing };
        try { hub.Save(); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            hub.Settings.Accounts[index] = previous;
            hub.ReportNotice("The monthly renewal date could not be saved. Check local settings permissions.");
            return;
        }
        hub.CheckBillingReminders(DateOnly.FromDateTime(DateTime.Now));
    }

    internal static Window EditorWindow(AccountProfile account)
    {
        var dialog = new Window
        {
            Title = "Monthly billing · AI SubHub", Width = 460, MaxHeight = 520, SizeToContent = SizeToContent.Height,
            CanResize = false, WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Padding = new Thickness(14), Background = Ui.Background, Icon = App.Icon()
        };
        var content = Ui.Stack(10);
        content.Children.Add(Ui.Heading(account.Label, 16));
        content.Children.Add(Ui.Text("Enter the next renewal or paid-access end date from your billing page. Providers do not expose a reliable billing date here.", 12, true));
        var enabled = new CheckBox { Content = "Track monthly billing", IsChecked = account.Billing is not null };
        content.Children.Add(enabled);
        var fields = Ui.Stack(8);
        fields.Children.Add(Ui.Text("Next renewal / end date (local calendar)", 12));
        var today = DateOnly.FromDateTime(DateTime.Now);
        var displayedDate = account.Billing is { } tracked ? BillingSchedule.NextDate(tracked, today) : (DateOnly?)null;
        var date = new CalendarDatePicker { SelectedDate = displayedDate?.ToDateTime(TimeOnly.MinValue), HorizontalAlignment = HorizontalAlignment.Stretch };
        Avalonia.Automation.AutomationProperties.SetName(date, "Next monthly renewal or access end date");
        fields.Children.Add(date);
        if (account.Billing is { } original && displayedDate != original.Date)
            fields.Children.Add(Ui.Text($"Estimated from your original monthly date, {original.Date:MMM d, yyyy}. Saving without changing the date keeps that monthly anchor.", 11, true));
        var renew = new CheckBox { Content = "Renews every month", IsChecked = account.Billing?.AutoRenew ?? true };
        fields.Children.Add(renew);
        fields.Children.Add(Ui.Text("Uncheck after canceling: this becomes the access end date, not a recurring renewal. SubHub never cancels or changes a subscription.", 11, true));
        var reminders = new CheckBox { Content = "Remind me 7 and 3 days before", IsChecked = account.Billing?.Reminders ?? true };
        fields.Children.Add(reminders);
        fields.Children.Add(Ui.Text("Reminders appear while SubHub runs, including in the tray. Missed reminders appear on the next start within the reminder window. Later monthly dates are estimates; verify them with your provider.", 11, true));
        fields.IsVisible = enabled.IsChecked == true;
        enabled.IsCheckedChanged += (_, _) => fields.IsVisible = enabled.IsChecked == true;
        content.Children.Add(fields);
        var error = Ui.Text("", 11); error.IsVisible = false; content.Children.Add(error);
        var buttons = Ui.Row(6); buttons.HorizontalAlignment = HorizontalAlignment.Right;
        buttons.Children.Add(Ui.Button("Cancel", () => dialog.Close()));
        buttons.Children.Add(Ui.Button("Save", () =>
        {
            MonthlyBilling? billing = null;
            if (enabled.IsChecked == true)
            {
                if (date.SelectedDate is not { } selected)
                {
                    error.Text = "Choose the date shown by your billing provider."; error.IsVisible = true; return;
                }
                var chosen = DateOnly.FromDateTime(selected);
                if (renew.IsChecked == true && chosen < today)
                {
                    error.Text = "Use the next renewal date, today or later."; error.IsVisible = true; return;
                }
                var same = account.Billing is { } previous && displayedDate == chosen && previous.AutoRenew == (renew.IsChecked == true);
                billing = new(same ? account.Billing!.Date : chosen, renew.IsChecked == true, reminders.IsChecked == true,
                    same ? account.Billing!.LastReminderDate : null, same ? account.Billing!.LastReminderDays : null);
            }
            dialog.Close(new MonthlyBillingEdit(billing));
        }));
        content.Children.Add(buttons);
        dialog.Content = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled };
        return dialog;
    }

    private sealed record MonthlyBillingEdit(MonthlyBilling? Billing);
}
