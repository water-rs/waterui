// water-rs/waterui#1262 competitive benchmark contestant: WinUI 3.
// Canonical workload spec (benchmarks/competitive/README.md): the same
// constants the shared apps/* contestants render. BENCH_WORKLOAD env var
// selects w1|w2|w3|w4; a missing or unrecognized value traps.
using System;
using System.Collections.Generic;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.UI;

namespace BenchWinUI;

public sealed partial class MainWindow : Window
{
    private const int RowCount = 10000;
    private const int RectCount = 200;
    private const int ParaCount = 50;
    private const double FieldW = 720;
    private const double FieldH = 440;
    private const double Rect = 40;

    // Canonical palette — identical in every contestant.
    private static readonly Color[] Palette =
    {
        Color.FromArgb(255, 0x3B, 0x82, 0xF6), Color.FromArgb(255, 0x10, 0xB9, 0x81),
        Color.FromArgb(255, 0xF5, 0x9E, 0x0B), Color.FromArgb(255, 0xEF, 0x44, 0x44),
        Color.FromArgb(255, 0x8B, 0x5C, 0xF6), Color.FromArgb(255, 0xEC, 0x48, 0x99),
    };

    public MainWindow()
    {
        InitializeComponent();
        Title = "bench-winui3";
        var workload = Environment.GetEnvironmentVariable("BENCH_WORKLOAD");
        Root.Children.Add(workload switch
        {
            "w1" => BuildHello(),
            "w2" => BuildFeed(),
            "w3" => BuildMotion(),
            "w4" => BuildText(),
            _ => throw new InvalidOperationException(
                $"missing or unrecognized BENCH_WORKLOAD '{workload}'; expected w1..w4"),
        });
    }

    private static string FeedTitle(int i) => $"Row title {i}";
    private static string FeedSubtitle(int i) => $"Second line of subtitle for item {i}";
    private static string FeedTimestamp(int i) =>
        $"{(i / 60) % 24:D2}:{i % 60:D2}";
    private static Color FeedColor(int i) => Palette[i % Palette.Length];

    // Canonical W4 text — benchmarks/competitive/lib/paragraphs.txt,
    // embedded (an app cannot read the suite's file at runtime).
    private static readonly string[] Paragraphs =
    {
        "The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.",
        "WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊",
        "Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.",
        "Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.",
        "How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.",
        "Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.",
        "ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏",
        "Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.",
        "The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.",
        "Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.",
    };

    // W1 Hello: centred label and a button that increments a counter.
    private UIElement BuildHello()
    {
        var count = 0;
        var countText = new TextBlock { Text = "Count: 0" };
        var button = new Button { Content = "Increment" };
        button.Click += (_, _) => countText.Text = $"Count: {++count}";
        var stack = new StackPanel
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Spacing = 16,
        };
        countText.FontSize = 20;
        stack.Children.Add(countText);
        stack.Children.Add(button);
        return stack;
    }

    // W2 Feed: lazily built list of 10,000 rows (ListView UI-virtualizes).
    private sealed class FeedRow
    {
        public required int Index { get; init; }
        public string Title => FeedTitle(Index);
        public string Subtitle => FeedSubtitle(Index);
        public string Timestamp => FeedTimestamp(Index);
        public Brush AvatarBrush => new SolidColorBrush(FeedColor(Index));
    }

    private UIElement BuildFeed()
    {
        var rows = new List<FeedRow>(RowCount);
        for (var i = 0; i < RowCount; i++)
            rows.Add(new FeedRow { Index = i });

        var itemTemplate = (DataTemplate)XamlReader.Load("""
            <DataTemplate xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation">
                <Grid Padding="16,10">
                    <Grid.ColumnDefinitions>
                        <ColumnDefinition Width="Auto"/>
                        <ColumnDefinition Width="*"/>
                        <ColumnDefinition Width="Auto"/>
                    </Grid.ColumnDefinitions>
                    <Ellipse Width="40" Height="40" Fill="{Binding AvatarBrush}"
                             VerticalAlignment="Center"/>
                    <StackPanel Grid.Column="1" VerticalAlignment="Center" Margin="12,0,0,0">
                        <TextBlock Text="{Binding Title}" FontSize="16"/>
                        <TextBlock Text="{Binding Subtitle}" FontSize="13" Foreground="#666666"/>
                    </StackPanel>
                    <TextBlock Grid.Column="2" Text="{Binding Timestamp}"
                               FontSize="13" Foreground="#888888" VerticalAlignment="Center"/>
                </Grid>
            </DataTemplate>
            """);

        return new ListView
        {
            ItemsSource = rows,
            ItemTemplate = itemTemplate,
        };
    }

    // xorshift64 — the shared per-rect random stream.
    private sealed class XorShift64
    {
        private ulong s;
        public XorShift64(ulong seed) => s = seed;
        public double Next01()
        {
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            return (s % 10000) / 10000.0;
        }
    }

    // W3 Motion: 200 rects wander a fixed 720x440 field, each on its own
    // xorshift64 stream; every duration tick picks new position/rotation/
    // opacity targets animated ease-in-out over (1200 + (i%5)*200) ms.
    private sealed class Wanderer
    {
        public Border Wall = null!;
        public XorShift64 Rng = null!;
        public int DurMs;
        public long TickMs;
        public double Fx, Fy, Fr, Fo;
        public double Tx, Ty, Tr, To;
        public double Cx, Cy, Cr, Co;
    }

    private static double EaseInOut(double f) =>
        f < 0.5 ? 4.0 * f * f * f : 1.0 - Math.Pow(-2.0 * f + 2.0, 3.0) / 2.0;

    private UIElement BuildMotion()
    {
        var canvas = new Canvas { Width = FieldW, Height = FieldH };
        canvas.HorizontalAlignment = HorizontalAlignment.Center;
        canvas.VerticalAlignment = VerticalAlignment.Center;
        var ws = new Wanderer[RectCount];
        var start = Environment.TickCount64;
        for (var i = 0; i < RectCount; i++)
        {
            var init = new XorShift64(
                0xD1B54A32D192ED03UL ^ (ulong)i * 0x2545F4914F6CDD1DUL);
            var w = new Wanderer
            {
                Cx = init.Next01() * (FieldW - Rect),
                Cy = init.Next01() * (FieldH - Rect),
                Cr = init.Next01() * 360,
                Co = 0.3 + init.Next01() * 0.7,
                Rng = new XorShift64(
                    0x9E3779B97F4A7C15UL ^ (ulong)i * 0xBF58476D1CE4E5B9UL),
                DurMs = 1200 + (i % 5) * 200,
                TickMs = start,
            };
            w.Wall = new Border
            {
                Width = Rect,
                Height = Rect,
                CornerRadius = new CornerRadius(10),
                Background = new SolidColorBrush(Palette[i % Palette.Length]),
                RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5),
                RenderTransform = new CompositeTransform(),
            };
            Retarget(w);
            Canvas.SetLeft(w.Wall, w.Cx);
            Canvas.SetTop(w.Wall, w.Cy);
            canvas.Children.Add(w.Wall);
            ws[i] = w;
        }

        CompositionTarget.Rendering += (_, _) =>
        {
            var now = Environment.TickCount64;
            foreach (var w in ws)
            {
                if (now - w.TickMs >= w.DurMs)
                {
                    w.Fx = w.Cx; w.Fy = w.Cy; w.Fr = w.Cr; w.Fo = w.Co;
                    Retarget(w);
                }
                var f = Math.Min(1.0, (now - w.TickMs) / (double)w.DurMs);
                var e = EaseInOut(f);
                w.Cx = w.Fx + (w.Tx - w.Fx) * e;
                w.Cy = w.Fy + (w.Ty - w.Fy) * e;
                w.Cr = w.Fr + (w.Tr - w.Fr) * e;
                w.Co = w.Fo + (w.To - w.Fo) * e;
                Canvas.SetLeft(w.Wall, w.Cx);
                Canvas.SetTop(w.Wall, w.Cy);
                ((CompositeTransform)w.Wall.RenderTransform).Rotation = w.Cr;
                w.Wall.Opacity = w.Co;
            }
        };
        return canvas;
    }

    private static void Retarget(Wanderer w)
    {
        w.Tx = w.Rng.Next01() * (FieldW - Rect);
        w.Ty = w.Rng.Next01() * (FieldH - Rect);
        w.Tr = w.Rng.Next01() * 360;
        w.To = 0.3 + w.Rng.Next01() * 0.7;
        w.TickMs = Environment.TickCount64;
    }

    // W4 Text: scrolling screen of 50 paragraphs of mixed Latin/CJK/emoji.
    private UIElement BuildText()
    {
        var stack = new StackPanel { Spacing = 6 };
        for (var p = 0; p < ParaCount; p++)
        {
            stack.Children.Add(new TextBlock
            {
                Text = Paragraphs[p % Paragraphs.Length],
                FontSize = 16,
                TextWrapping = TextWrapping.Wrap,
                Margin = new Thickness(16, 10, 16, 10),
            });
        }
        return new ScrollViewer
        {
            Content = stack,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
        };
    }
}
