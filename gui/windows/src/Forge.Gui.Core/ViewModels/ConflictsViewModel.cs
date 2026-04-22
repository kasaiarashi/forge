using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Forge.Gui.Core.Models;

namespace Forge.Gui.Core.ViewModels;

public sealed partial class PropertyConflictViewModel : ObservableObject
{
    [ObservableProperty] private string _propertyPath = string.Empty;
    [ObservableProperty] private string? _baseValue;
    [ObservableProperty] private string? _ourValue;
    [ObservableProperty] private string? _theirValue;
    [ObservableProperty] private ConflictChoice _choice = ConflictChoice.Unresolved;
    [ObservableProperty] private string? _customValue;

    public bool ChooseOurs
    {
        get => Choice == ConflictChoice.TakeOurs;
        set { if (value) Choice = ConflictChoice.TakeOurs; }
    }

    public bool ChooseTheirs
    {
        get => Choice == ConflictChoice.TakeTheirs;
        set { if (value) Choice = ConflictChoice.TakeTheirs; }
    }

    public bool ChooseCustom
    {
        get => Choice == ConflictChoice.Custom;
        set { if (value) Choice = ConflictChoice.Custom; }
    }

    partial void OnChoiceChanged(ConflictChoice value)
    {
        OnPropertyChanged(nameof(ChooseOurs));
        OnPropertyChanged(nameof(ChooseTheirs));
        OnPropertyChanged(nameof(ChooseCustom));
    }
}

public sealed partial class FileConflictViewModel : ObservableObject
{
    [ObservableProperty] private string _path = string.Empty;
    [ObservableProperty] private bool _isBinary;
    public ObservableCollection<PropertyConflictViewModel> Properties { get; } = new();

    public bool IsResolved => Properties.All(p => p.Choice != ConflictChoice.Unresolved);
}

public sealed partial class ConflictsViewModel : ObservableObject
{
    public ObservableCollection<FileConflictViewModel> Files { get; } = new();

    [ObservableProperty] private FileConflictViewModel? _selected;
    [ObservableProperty] private int _resolvedCount;
    [ObservableProperty] private int _totalCount;

    public ConflictsViewModel()
    {
        LoadMockConflicts();
        Selected = Files.FirstOrDefault();
        TotalCount = Files.Count;
    }

    [RelayCommand]
    public void MarkResolved()
    {
        ResolvedCount = Files.Count(f => f.IsResolved);
    }

    private void LoadMockConflicts()
    {
        Files.Add(MakeFile("Content/Characters/Hero.uasset", isBinary: true, props:
        [
            new("Properties.MaxHealth", "100", "125", "150"),
            new("Properties.Scale",     "(1,1,1)", "(1.2,1.2,1.2)", "(1,1.5,1)"),
            new("Properties.Material",  "M_Hero_v1", "M_Hero_v2", "M_Hero_v3"),
        ]));
        Files.Add(MakeFile("Source/DemoArena/Private/HeroChar.cpp", isBinary: false, props:
        [
            new("line 42", "Velocity = FVector(500, 0, 0);",
                           "Velocity = FVector(800, 0, 0);",
                           "Velocity = FVector(600, 0, 0);"),
        ]));
    }

    private static FileConflictViewModel MakeFile(string path, bool isBinary,
        (string PropertyPath, string? Base, string? Ours, string? Theirs)[] props)
    {
        var fc = new FileConflictViewModel { Path = path, IsBinary = isBinary };
        foreach (var p in props)
        {
            fc.Properties.Add(new PropertyConflictViewModel
            {
                PropertyPath = p.PropertyPath,
                BaseValue = p.Base,
                OurValue = p.Ours,
                TheirValue = p.Theirs,
            });
        }
        return fc;
    }
}
