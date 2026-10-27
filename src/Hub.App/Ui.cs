using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Styling;

namespace Hub.App;

internal static class Ui
{
    public static bool Dark => Application.Current?.ActualThemeVariant != ThemeVariant.Light;
    public static IBrush Muted => Brush(Dark ? "#A0ADC2" : "#526078");
    public static IBrush Accent => Brush(Dark ? "#A2BAFF" : "#345BCD");
    public static IBrush Surface => Brush(Dark ? "#1C2331" : "#FFFFFF");
    public static IBrush Background => Brush(Dark ? "#111722" : "#F2F5FA");
    public static IBrush Line => Brush(Dark ? "#323D51" : "#D8DFEB");
    public static IBrush Foreground => Brush(Dark ? "#F0F3FA" : "#172136");
    public static IBrush Brush(string color) => new SolidColorBrush(Color.Parse(color));
    public static TextBlock Text(string text, double size = 14, bool muted = false) => new() { Text = text, FontSize = size, Foreground = muted ? Muted : Foreground, TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
    public static TextBlock Heading(string text, double size = 24) => new() { Text = text, FontSize = size, FontWeight = FontWeight.SemiBold, TextWrapping = TextWrapping.Wrap };
    public static StackPanel Stack(double spacing = 12) => new() { Spacing = spacing };
    public static string AccountHint(string label, Hub.Core.UsageResult? result)
    {
        var email = result?.AccountEmail ?? "Unavailable; refresh after signing in.";
        var resets = result?.Resets is { } available ? $"\nManual usage resets: {available.AvailableCount} available" + (available.EarliestExpiry is { } expiry ? $"; earliest expires {expiry.LocalDateTime:g}" : "") : "";
        return $"{label}\nSigned-in email: {email}{resets}";
    }

    public static Control AccountName(string label, string plan, double size, double maximumNameWidth, string hint, string? resetText = null)
    {
        var row = new Grid { ColumnDefinitions = new ColumnDefinitions("Auto,Auto,Auto"), ColumnSpacing = 6, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Center };
        var name = Text(label, size); name.MaxWidth = maximumNameWidth; name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
        ToolTip.SetTip(name, hint); row.Children.Add(name);
        var text = Text(plan, 10); text.Foreground = Accent; text.TextWrapping = TextWrapping.NoWrap;
        var tag = new Border { Child = text, BorderBrush = Line, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(4), Padding = new Thickness(5, 1), VerticalAlignment = VerticalAlignment.Center };
        ToolTip.SetTip(tag, "Latest available provider plan, with saved login metadata as fallback. Not a live billing check. Refresh to check for a changed plan.");
        Grid.SetColumn(tag, 1); row.Children.Add(tag);
        if (resetText is not null)
        {
            var resetLabel = Text(resetText, 10); resetLabel.TextWrapping = TextWrapping.NoWrap; resetLabel.Foreground = Accent;
            var reset = new Border { Child = resetLabel, BorderBrush = Line, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(4), Padding = new Thickness(5, 1), VerticalAlignment = VerticalAlignment.Center };
            ToolTip.SetTip(reset, hint + "\nAvailability is from the last successful usage read. Reset redemption stays in the provider's app.");
            Grid.SetColumn(reset, 2); row.Children.Add(reset);
        }
        return row;
    }
    public static StackPanel Row(double spacing = 10) => new() { Orientation = Orientation.Horizontal, Spacing = spacing, VerticalAlignment = VerticalAlignment.Center };
    public static Button Button(string label, Action action, string? hint = null)
    {
        var button = new Button { Content = label, FontSize = 12, MinHeight = 26, Padding = new Thickness(8, 4), Background = Brushes.Transparent, CornerRadius = new CornerRadius(5), Cursor = new Avalonia.Input.Cursor(Avalonia.Input.StandardCursorType.Hand), BorderThickness = new Thickness(1), BorderBrush = Line };
        button.Click += (_, _) => action();
        if (hint is not null) ToolTip.SetTip(button, hint);
        return button;
    }
    public static Button AsyncButton(string label, Func<Task> action)
    {
        var button = Button(label, () => { });
        button.Click += async (_, _) => await action();
        return button;
    }
    public static Border Card(Control content) => new() { Child = content, Background = Surface, BorderBrush = Line, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12), Padding = new Thickness(18) };
    public static Control Header(string title, Control actions)
    {
        var grid = new Grid { ColumnDefinitions = new ColumnDefinitions("*,Auto"), ColumnSpacing = 14 };
        grid.Children.Add(Heading(title, 18)); Grid.SetColumn(actions, 1); grid.Children.Add(actions); return grid;
    }
    public static string ResetText(DateTimeOffset? reset)
    {
        if (reset is null) return "Reset time unavailable";
        var remaining = reset.Value - DateTimeOffset.UtcNow;
        if (remaining <= TimeSpan.Zero) return "Reset due · refresh to confirm";
        var relative = remaining.TotalDays >= 1 ? $"{(int)remaining.TotalDays}d {remaining.Hours}h" : remaining.TotalHours >= 1 ? $"{(int)remaining.TotalHours}h {remaining.Minutes}m" : $"{Math.Max(1, remaining.Minutes)}m";
        return $"Resets in {relative} · {reset.Value.LocalDateTime:ddd HH:mm}";
    }
    public static string Age(DateTimeOffset at)
    {
        var age = DateTimeOffset.UtcNow - at;
        return age.TotalMinutes < 1 ? "just now" : age.TotalHours < 1 ? $"{Math.Max(1, (int)age.TotalMinutes)}m ago" : age.TotalDays < 1 ? $"{(int)age.TotalHours}h ago" : $"{(int)age.TotalDays}d ago";
    }
}
