using Forge.Gui.App.Services;
using Forge.Gui.Core.Services;
using Forge.Gui.Core.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;

namespace Forge.Gui.App;

public partial class App : Application
{
    public static IServiceProvider Services { get; private set; } = null!;
    public Window? MainWindow { get; private set; }

    public App()
    {
        InitializeComponent();
        Services = BuildServices();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        // Load settings first so the HomePage has its RecentRepos list
        // populated before it first renders.
        _ = Services.GetRequiredService<ISettingsStore>().LoadAsync();

        MainWindow = new MainWindow();
        MainWindow.Activate();
    }

    private static IServiceProvider BuildServices()
    {
        var services = new ServiceCollection();

        // Default client is NoWorkspaceClient (empty state). OpenRepoService
        // swaps it to a real FfiForgeClient once the user picks a folder
        // (or the last-opened repo auto-reopens on launch).
        services.AddSingleton<NoWorkspaceClient>();
        services.AddSingleton<IForgeClient, ActiveForgeClient>(sp =>
            new ActiveForgeClient(sp.GetRequiredService<NoWorkspaceClient>()));
        services.AddSingleton<ActiveForgeClient>(sp =>
            (ActiveForgeClient)sp.GetRequiredService<IForgeClient>());

        services.AddSingleton<IUiModeService, UiModeService>();
        services.AddSingleton<IRepoRegistry, InMemoryRepoRegistry>();
        services.AddSingleton<ISettingsStore, JsonSettingsStore>();
        services.AddSingleton<IWorkspaceWatcher, WorkspaceWatcher>();
        services.AddSingleton<OpenRepoService>(sp => new OpenRepoService(
            sp.GetRequiredService<ActiveForgeClient>(),
            sp.GetRequiredService<IRepoRegistry>(),
            sp.GetRequiredService<ISettingsStore>(),
            sp.GetRequiredService<NoWorkspaceClient>()));

        services.AddTransient<ShellViewModel>();
        services.AddTransient<HomeViewModel>();
        services.AddTransient<ChangesViewModel>();
        services.AddTransient<HistoryViewModel>();
        services.AddTransient<LocksViewModel>();
        services.AddTransient<BranchesViewModel>();
        services.AddTransient<SettingsViewModel>();
        services.AddTransient<CloneViewModel>();
        services.AddTransient<ConflictsViewModel>();
        services.AddTransient<DiffViewModel>();

        return services.BuildServiceProvider();
    }
}
