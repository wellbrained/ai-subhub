using System.Globalization;

namespace Hub.Core;

public sealed record MonthlyBilling(DateOnly Date, bool AutoRenew = true, bool Reminders = true,
    DateOnly? LastReminderDate = null, int? LastReminderDays = null);

public sealed record BillingReminder(AccountProfile Account, DateOnly Date, int DaysRemaining, int Threshold);

public static class BillingSchedule
{
    public static DateOnly NextDate(MonthlyBilling billing, DateOnly today)
    {
        if (!billing.AutoRenew || billing.Date >= today) return billing.Date;
        var months = (today.Year - billing.Date.Year) * 12 + today.Month - billing.Date.Month;
        var date = billing.Date.AddMonths(months);
        return date >= today || date.Year == 9999 && date.Month == 12 ? date : billing.Date.AddMonths(months + 1);
    }

    public static string Description(MonthlyBilling billing, DateOnly today)
    {
        var date = NextDate(billing, today);
        var days = date.DayNumber - today.DayNumber;
        var verb = !billing.AutoRenew && days < 0 ? "Ended" : billing.AutoRenew ? "Renews" : "Ends";
        var relative = days == 0 ? "today" : days == 1 ? "tomorrow" : days > 0 ? $"in {days} days" : $"{-days} days ago";
        var source = date == billing.Date ? "manual" : "estimated";
        return $"{verb} {date.ToString("MMM d, yyyy", CultureInfo.CurrentCulture)} · {relative} · {source}";
    }

    public static BillingReminder? Reminder(AccountProfile account, DateOnly today)
    {
        if (account.Billing is not { Reminders: true } billing) return null;
        var date = NextDate(billing, today);
        var days = date.DayNumber - today.DayNumber;
        if (days is < 0 or > 7) return null;
        var threshold = days <= 3 ? 3 : 7;
        if (billing.LastReminderDate == date && billing.LastReminderDays <= threshold) return null;
        return new(account, date, days, threshold);
    }

    public static string BillingUrl(ProviderKind provider) => provider switch
    {
        ProviderKind.Claude => "https://claude.ai/settings/billing",
        ProviderKind.Codex => "https://chatgpt.com/#settings/Account",
        _ => throw new ArgumentOutOfRangeException(nameof(provider))
    };
}
