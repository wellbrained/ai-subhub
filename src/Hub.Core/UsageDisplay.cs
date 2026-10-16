namespace Hub.Core;

public static class UsageDisplay
{
    public static double Value(QuotaWindow window, string style) => style == "Used" ? window.UsedPercent : window.RemainingPercent;
    public static string Text(QuotaWindow window, string style) => $"{Value(window, style):0.#}% {(style == "Used" ? "used" : "remaining")}";
}
