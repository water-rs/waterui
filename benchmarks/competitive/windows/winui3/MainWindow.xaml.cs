// water-rs/waterui#1262 competitive benchmark contestant: WinUI 3.
// Canonical workload spec (benchmarks/competitive/WORKLOADS.md): the same
// constants the shared apps/* contestants render. BENCH_WORKLOAD env var
// selects w1..w6; a missing or unrecognized value traps. Ladder workloads
// (w5, w6) additionally require BENCH_STEP naming a ladder member.
using System;
using System.Collections.Generic;
using System.Numerics;
using Microsoft.UI;
using Microsoft.UI.Composition;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
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
            "w3" => BuildMotion(RectCount),
            "w4" => BuildText(),
            "w5" => BuildMotion(LadderStep(workload)),
            "w6" => BuildFeed(LadderStep(workload)),
            _ => throw new InvalidOperationException(
                $"missing or unrecognized BENCH_WORKLOAD '{workload}'; expected w1..w6"),
        });
    }

    // BENCH_STEP pins one ladder step per launch — a missing or
    // out-of-ladder step is a hard failure, never a silent default.
    private static readonly int[] W5Ladder = { 200, 400, 800, 1600, 3200, 6400, 12800, 25600 };
    private static readonly int[] W6Ladder = { 1, 2, 4, 8, 16, 32, 64 };

    private static int LadderStep(string workload)
    {
        var ladder = workload == "w6" ? W6Ladder : W5Ladder;
        var s = Environment.GetEnvironmentVariable("BENCH_STEP");
        if (string.IsNullOrEmpty(s))
            throw new InvalidOperationException(
                $"missing BENCH_STEP for {workload}; expected one of the declared ladder members");
        if (!int.TryParse(s, out var n))
            throw new InvalidOperationException(
                $"malformed BENCH_STEP '{s}'; expected an integer");
        if (Array.IndexOf(ladder, n) < 0)
            throw new InvalidOperationException(
                $"unrecognized BENCH_STEP {n}; expected one of the {workload} ladder");
        return n;
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

    // W2 Feed / W6 Feed capacity: lazily built list of 10,000 rows
    // (ListView UI-virtualizes). W6's `complexity` adds that many sibling
    // cells — a 14x14 rounded square (radius 4.2 ≈ ratio 0.3) over a
    // "c{j}" caption — between the text column and the timestamp, cells
    // separated by 4, the group keeping the row's standard 12 gap.
    private sealed class CellDef
    {
        public required Brush Square { get; init; }
        public required string Label { get; init; }
    }

    private sealed class FeedRow
    {
        public required int Index { get; init; }
        public int Complexity { get; init; }
        public string Title => FeedTitle(Index);
        public string Subtitle => FeedSubtitle(Index);
        public string Timestamp => FeedTimestamp(Index);
        public Brush AvatarBrush => new SolidColorBrush(FeedColor(Index));
        public List<CellDef> Cells
        {
            get
            {
                var cells = new List<CellDef>(Complexity);
                for (var j = 0; j < Complexity; j++)
                    cells.Add(new CellDef
                    {
                        Square = new SolidColorBrush(
                            Palette[(Index + j) % Palette.Length]),
                        Label = $"c{j}",
                    });
                return cells;
            }
        }
    }

    private UIElement BuildFeed(int complexity = 0)
    {
        var rows = new List<FeedRow>(RowCount);
        for (var i = 0; i < RowCount; i++)
            rows.Add(new FeedRow { Index = i, Complexity = complexity });

        var itemTemplate = (DataTemplate)XamlReader.Load("""
            <DataTemplate xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation">
                <Grid Padding="16,10">
                    <Grid.ColumnDefinitions>
                        <ColumnDefinition Width="Auto"/>
                        <ColumnDefinition Width="*"/>
                        <ColumnDefinition Width="Auto"/>
                        <ColumnDefinition Width="Auto"/>
                    </Grid.ColumnDefinitions>
                    <Ellipse Width="40" Height="40" Fill="{Binding AvatarBrush}"
                             VerticalAlignment="Center"/>
                    <StackPanel Grid.Column="1" VerticalAlignment="Center" Margin="12,0,0,0" Spacing="4">
                        <TextBlock Text="{Binding Title}" FontSize="16"/>
                        <TextBlock Text="{Binding Subtitle}" FontSize="13" Foreground="#666666"/>
                    </StackPanel>
                    <ItemsControl Grid.Column="2" ItemsSource="{Binding Cells}"
                                  VerticalAlignment="Center" Margin="12,0,12,0">
                        <ItemsControl.ItemsPanel>
                            <ItemsPanelTemplate>
                                <StackPanel Orientation="Horizontal" Spacing="4"/>
                            </ItemsPanelTemplate>
                        </ItemsControl.ItemsPanel>
                        <ItemsControl.ItemTemplate>
                            <DataTemplate>
                                <StackPanel HorizontalAlignment="Center">
                                    <Border Width="14" Height="14" CornerRadius="4.2"
                                            Background="{Binding Square}"
                                            HorizontalAlignment="Center"/>
                                    <TextBlock Text="{Binding Label}" FontSize="12"/>
                                </StackPanel>
                            </DataTemplate>
                        </ItemsControl.ItemTemplate>
                    </ItemsControl>
                    <TextBlock Grid.Column="3" Text="{Binding Timestamp}"
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
        public Visual Visual = null!;
        public XorShift64 Rng = null!;
        public int DurMs;
        public double Tx, Ty, Tr, To;
        public double Cx, Cy, Cr, Co;
    }

    private UIElement BuildMotion(int count)
    {
        var canvas = new Canvas { Width = FieldW, Height = FieldH };
        canvas.HorizontalAlignment = HorizontalAlignment.Center;
        canvas.VerticalAlignment = VerticalAlignment.Center;
        var compositor =
            ElementCompositionPreview.GetElementVisual(canvas).Compositor;
        var ws = new Wanderer[count];
        for (var i = 0; i < count; i++)
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
            };
            w.Wall = new Border
            {
                Width = Rect,
                Height = Rect,
                CornerRadius = new CornerRadius(10),
                Background = new SolidColorBrush(Palette[i % Palette.Length]),
            };
            w.Visual = ElementCompositionPreview.GetElementVisual(w.Wall);
            // rects render at their seeded pose before the first segment —
            // never from the zero transform. CenterPoint puts rotation
            // about the rect's centre; Offset places its centre.
            w.Visual.CenterPoint = new Vector3((float)(Rect / 2.0),
                                               (float)(Rect / 2.0), 0f);
            w.Visual.Offset = new Vector3((float)(w.Cx + Rect / 2.0),
                                          (float)(w.Cy + Rect / 2.0), 0f);
            w.Visual.RotationAngleInDegrees = (float)w.Cr;
            w.Visual.Opacity = (float)w.Co;
            canvas.Children.Add(w.Wall);
            ws[i] = w;
        }
        foreach (var w in ws)
        {
            Retarget(w);
            StartSegment(compositor, w);
        }
        return canvas;
    }

    // Each segment runs on the compositor as keyframe animations under
    // the spec's cubic-bezier(0.42, 0, 0.58, 1) easing — offset, rotation
    // and opacity. The scoped batch's Completed handler (UI thread)
    // syncs the model to the landed pose and retargets the next segment,
    // so motion never drives per-frame property writes from the UI
    // thread (no CompositionTarget.Rendering).
    private static void StartSegment(Compositor compositor, Wanderer w)
    {
        var easing = compositor.CreateCubicBezierEasingFunction(
            new Vector2(0.42f, 0f), new Vector2(0.58f, 1f));
        var dur = TimeSpan.FromMilliseconds(w.DurMs);
        var offset = compositor.CreateVector3KeyFrameAnimation();
        offset.InsertKeyFrame(1f,
            new Vector3((float)(w.Tx + Rect / 2.0),
                        (float)(w.Ty + Rect / 2.0), 0f), easing);
        offset.Duration = dur;
        var rot = compositor.CreateScalarKeyFrameAnimation();
        rot.InsertKeyFrame(1f, (float)w.Tr, easing);
        rot.Duration = dur;
        var op = compositor.CreateScalarKeyFrameAnimation();
        op.InsertKeyFrame(1f, (float)w.To, easing);
        op.Duration = dur;
        var batch = compositor.CreateScopedBatch(CompositionBatchTypes.Animation);
        w.Visual.StartAnimation(nameof(w.Visual.Offset), offset);
        w.Visual.StartAnimation(nameof(w.Visual.RotationAngleInDegrees), rot);
        w.Visual.StartAnimation(nameof(w.Visual.Opacity), op);
        batch.Completed += (_, _) =>
        {
            w.Cx = w.Tx; w.Cy = w.Ty; w.Cr = w.Tr; w.Co = w.To;
            Retarget(w);
            StartSegment(compositor, w);
        };
        batch.End();
    }

    private static void Retarget(Wanderer w)
    {
        w.Tx = w.Rng.Next01() * (FieldW - Rect);
        w.Ty = w.Rng.Next01() * (FieldH - Rect);
        w.Tr = w.Rng.Next01() * 360;
        w.To = 0.3 + w.Rng.Next01() * 0.7;
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
