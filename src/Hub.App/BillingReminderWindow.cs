using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Hub.Core;

namespace Hub.App;

internal sealed class BillingReminderWindow : Window
{
    private readonly HubController _hub;
    private readonly Dictionary<Guid, BillingReminder> _reminders = [];
    private readonly StackPanel _content = Ui.Stack(12);

    internal BillingReminderWindow(HubController hub)
    {
        _hub = hub;
        Title = "Renewal reminder · AI SubHub"; Width = 460; MaxHeight = 500; SizeToContent = SizeToContent.Height;
        Background = Ui.Background; Icon = App.Icon(); Padding = new Thickness(16);
        Content = new ScrollViewer { Content = _content, HorizontalScrollBarVisibility = Avalonia.Controls.Primitives.ScrollBarVisibility.Disabled };
    }

    internal void Add(IReadOnlyList<BillingReminder> reminders)
    {
        foreach (var reminder in reminders) _reminders[reminder.Account.Id] = reminder;
        _content.Children.Clear();
        _content.Children.Add(Ui.Heading("Upcoming monthly billing", 18));
        _content.Children.Add(Ui.Text("Decide whether to keep or cancel with your provider. These reminders use your manually entered dates, not a billing confirmation.", 12, true));
        foreach (var reminder in _reminders.Values)
        {
            var row = Ui.Stack(5);
            row.Children.Add(Ui.Heading($"{reminder.Account.Label} · {reminder.Account.Provider}", 14));
            var kind = reminder.Account.Billing!.AutoRenew ? "Renewal" : "Access end";
            var source = reminder.Date == reminder.Account.Billing.Date ? "manual" : "estimated";
            row.Children.Add(Ui.Text($"{kind} date: {reminder.Date:MMM d, yyyy} · {source}", 12));
            var open = Ui.AsyncButton("Open billing", () => MonthlyBillingUi.OpenBilling(this, _hub, reminder.Account));
            ToolTip.SetTip(open, MonthlyBillingUi.BillingHint); row.Children.Add(open);
            _content.Children.Add(row);
        }
        _content.Children.Add(Ui.Text("Verify the date and signed-in account in your browser. Canceling, if you choose to, stays with the provider or app store.", 11, true));
        var dismiss = Ui.Button("Dismiss", Close); dismiss.HorizontalAlignment = HorizontalAlignment.Right;
        _content.Children.Add(dismiss);
    }
}
