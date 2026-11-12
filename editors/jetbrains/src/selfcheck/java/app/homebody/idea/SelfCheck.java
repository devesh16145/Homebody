package app.homebody.idea;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.intellij.notification.Notification;
import com.intellij.notification.NotificationsManager;
import com.intellij.openapi.actionSystem.ActionGroup;
import com.intellij.openapi.actionSystem.ActionManager;
import com.intellij.openapi.actionSystem.ActionPlaces;
import com.intellij.openapi.actionSystem.AnAction;
import com.intellij.openapi.actionSystem.AnActionEvent;
import com.intellij.openapi.actionSystem.DefaultActionGroup;
import com.intellij.openapi.actionSystem.impl.SimpleDataContext;
import com.intellij.openapi.application.ApplicationInfo;
import com.intellij.openapi.application.ApplicationManager;
import com.intellij.openapi.project.Project;
import com.intellij.openapi.wm.StatusBar;
import com.intellij.openapi.wm.StatusBarWidget;
import com.intellij.openapi.wm.WindowManager;

import javax.swing.AbstractButton;
import javax.swing.JButton;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.SwingUtilities;
import javax.swing.text.JTextComponent;
import java.awt.Component;
import java.awt.Container;
import java.awt.Window;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.function.Supplier;

/**
 * The plugin, exercised inside the running editor against the Homebody on this computer.
 *
 * Not part of a released build (see build.gradle.kts). It stands in for the person: it runs the
 * plugin's own actions and presses the buttons of the dialogs they really show. Everything
 * else is real: the status bar entry, the notifications, the engine, the fence. It writes what
 * it found as JSON and undoes what it did (the project it brought in is removed, the agent it
 * opened is stopped, "block everything" is released).
 */
public final class SelfCheck {
    private static final String CLI = "/Applications/Homebody.app/Contents/MacOS/homebody";
    private final Project project;
    private final Path out;
    private final String agent = System.getProperty("homebody.selfcheck.agent", "claude-code");
    private final JsonArray results = new JsonArray();
    private final List<String> dialogs = new ArrayList<>();

    private SelfCheck(Project project, String out) {
        this.project = project;
        this.out = Path.of(out);
    }

    public static void start(Project project, String out) {
        Thread t = new Thread(() -> new SelfCheck(project, out).run(), "homebody-selfcheck");
        t.setDaemon(true);
        t.start();
    }

    private void check(String name, boolean ok, Object detail) {
        JsonObject r = new JsonObject();
        r.addProperty("name", name);
        r.addProperty("ok", ok);
        String d = String.valueOf(detail == null ? "" : detail);
        r.addProperty("detail", d.length() > 400 ? d.substring(0, 400) : d);
        results.add(r);
    }

    private static void pause(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    private static boolean until(Supplier<Boolean> what, long ms) {
        long end = System.currentTimeMillis() + ms;
        while (System.currentTimeMillis() < end) {
            try {
                if (Boolean.TRUE.equals(what.get())) return true;
            } catch (RuntimeException ignored) {
                // not yet
            }
            pause(250);
        }
        return false;
    }

    private static JsonElement call(String cmd, JsonObject args) {
        try {
            return HomebodyClient.call(cmd, args);
        } catch (HomebodyClient.Refused e) {
            throw new IllegalStateException(e.getMessage());
        }
    }

    private static JsonObject status() {
        return call("get_status", null).getAsJsonObject();
    }

    private String widgetText() {
        StatusBar bar = WindowManager.getInstance().getStatusBar(project);
        StatusBarWidget w = bar == null ? null : bar.getWidget("HomebodyStatus");
        return w instanceof HomebodyWidgetFactory.Widget hw ? hw.getText() : null;
    }

    // ---- the dialogs the plugin shows, read and answered as a person would

    private static void collect(Component c, StringBuilder text, List<JButton> buttons) {
        if (c instanceof JButton b) buttons.add(b);
        if (c instanceof JLabel l && l.getText() != null) text.append(l.getText()).append('\n');
        else if (c instanceof JTextComponent t && t.getText() != null) text.append(t.getText()).append('\n');
        else if (c instanceof AbstractButton b && b.getText() != null) text.append(b.getText()).append('\n');
        if (c instanceof Container k) for (Component child : k.getComponents()) collect(child, text, buttons);
    }

    private static String plain(String html) {
        return html.replaceAll("(?s)<style.*?</style>", " ").replaceAll("(?s)<head.*?</head>", " ").replaceAll("<[^>]+>", " ").replace("&nbsp;", " ").replace("&#39;", "'").replace("&amp;", "&").replaceAll("\\s+", " ").trim();
    }

    /** Wait for a dialog whose words include `words`, press its `button`, and return what it said. */
    private String answer(String words, String button, long ms) {
        String[] said = {null};
        until(() -> {
            try {
                SwingUtilities.invokeAndWait(() -> {
                    for (Window w : Window.getWindows()) {
                        if (!w.isShowing() || !(w instanceof JDialog)) continue;
                        StringBuilder text = new StringBuilder();
                        List<JButton> buttons = new ArrayList<>();
                        collect(w, text, buttons);
                        String all = plain(text.toString());
                        if (!all.contains(words)) continue;
                        for (JButton b : buttons) {
                            if (b.getText() != null && b.isShowing() && b.getText().replace("&", "").replace("\u001B", "").equalsIgnoreCase(button)) {
                                said[0] = all;
                                b.doClick();
                                return;
                            }
                        }
                    }
                });
            } catch (Exception ignored) {
                // try again
            }
            return said[0] != null;
        }, ms);
        if (said[0] != null) dialogs.add(said[0]);
        return said[0];
    }

    private List<String> openDialogs() {
        List<String> open = new ArrayList<>();
        try {
            SwingUtilities.invokeAndWait(() -> {
                for (Window w : Window.getWindows()) {
                    if (!w.isShowing() || !(w instanceof JDialog)) continue;
                    StringBuilder text = new StringBuilder();
                    collect(w, text, new ArrayList<>());
                    open.add(plain(text.toString()));
                }
            });
        } catch (Exception ignored) {
            // nothing to report
        }
        return open;
    }

    private void perform(String actionId) {
        AnAction action = ActionManager.getInstance().getAction(actionId);
        ApplicationManager.getApplication().invokeLater(() ->
            action.actionPerformed(AnActionEvent.createFromAnAction(action, null, ActionPlaces.UNKNOWN, SimpleDataContext.getProjectContext(project))));
    }

    private static String sh(long seconds, String... command) {
        try {
            Process p = new ProcessBuilder(command).redirectErrorStream(true).start();
            byte[] all = p.getInputStream().readAllBytes();
            p.waitFor(seconds, TimeUnit.SECONDS);
            return new String(all, StandardCharsets.UTF_8).trim();
        } catch (IOException | InterruptedException e) {
            return "could not run: " + e.getMessage();
        }
    }

    /** Processes of a private user Homebody made (their names start with _hb) running `command`. */
    private static long fenced(String command) {
        return sh(10, "/bin/ps", "-axo", "user=,comm=").lines().filter(l -> l.startsWith("_hb") && l.contains(command)).count();
    }

    private void run() {
        JsonObject brought = null;
        try {
            // 1. It loads, and its status bar entry says what Homebody says.
            boolean there = until(() -> widgetText() != null && !"Homebody".equals(widgetText()), 40000);
            JsonObject status = status();
            check("the status bar has Homebody's entry", there, widgetText());
            check("it shows Homebody's state", HomebodyClient.summary(status).equals(widgetText()), widgetText() + " / " + HomebodyClient.summary(status));
            ActionManager am = ActionManager.getInstance();
            AnAction menu = am.getAction("Homebody.Menu");
            int inMenu = menu instanceof DefaultActionGroup g ? g.getChildActionsOrStubs().length : menu instanceof ActionGroup ? -1 : 0;
            boolean all = true;
            for (String id : new String[]{"Homebody.OpenAgent", "Homebody.ReviewChanges", "Homebody.ShowActivity", "Homebody.BlockEverything", "Homebody.Release"}) all &= am.getAction(id) != null;
            check("its five actions are in the Tools menu group", all && inMenu == 5, "group has " + inMenu);

            // 2. Recent activity.
            perform("Homebody.ShowActivity");
            JsonArray feed = call("get_feed", HomebodyClient.args("limit", 25)).getAsJsonArray();
            String firstApp = feed.size() > 0 ? HomebodyClient.text(feed.get(0).getAsJsonObject(), "appName") : "Nothing yet.";
            String activity = answer(firstApp, "OK", 15000);
            check("recent activity is shown", activity != null, "no dialog with " + firstApp);

            // 3. Open an agent protected, on a copy of this project. The project was opened by a
            //    path that goes through a link; it has to be recognised by its real path.
            String dir = Actions.folder(project);
            check("the project is known by its real path", dir != null && !dir.equals(project.getBasePath()) && Files.isDirectory(Path.of(dir)), project.getBasePath() + " -> " + dir);
            JsonObject app = null;
            for (JsonElement a : status.getAsJsonArray("apps")) if (HomebodyClient.text(a.getAsJsonObject(), "id").equals(agent)) app = a.getAsJsonObject();
            check("the agent is ready in Homebody", app != null && app.get("canStart").getAsBoolean(), agent);
            String name = HomebodyClient.text(app, "name"), command = HomebodyClient.text(app, "command");
            long before = fenced(command);
            perform("Homebody.OpenAgent");
            check("it asks which agent", answer("Which agent?", name, 15000) != null, openDialogs());
            String explained = answer("works on a copy of this project", "Bring This Project In", 15000);
            check("it explains the copy and asks before bringing the project in", explained != null, openDialogs());
            boolean in = until(() -> {
                for (JsonElement p : call("get_projects", HomebodyClient.args("appId", agent)).getAsJsonArray()) {
                    JsonObject o = p.getAsJsonObject();
                    if (HomebodyClient.text(o, "source").equals(dir) && o.get("copyExists").getAsBoolean()) return true;
                }
                return false;
            }, 20000);
            for (JsonElement p : call("get_projects", HomebodyClient.args("appId", agent)).getAsJsonArray()) {
                if (HomebodyClient.text(p.getAsJsonObject(), "source").equals(dir)) brought = p.getAsJsonObject();
            }
            check("the project was brought in as a copy", in && brought != null && Files.exists(Path.of(HomebodyClient.text(brought, "copy"), "hello.txt")), brought);
            boolean started = until(() -> fenced(command) > before, 25000);
            check("the agent was opened as its own private user, in a terminal", started, "no new process of a private user running " + command);
            call("app_stop", HomebodyClient.args("appId", agent));
            check("stopping it from Homebody ends it", until(() -> fenced(command) <= before, 15000), fenced(command) + " still running");

            // 4. The agent changes a file in its copy (run as the agent); review it and take it.
            if (brought != null) {
                String copy = HomebodyClient.text(brought, "copy");
                String wrote = sh(30, CLI, "run", agent, "--cwd", copy, "--", "/bin/sh", "-c", "echo written by the agent > from-agent.txt");
                check("the agent can write in its copy, and not in your project", Files.exists(Path.of(copy, "from-agent.txt")) && !Files.exists(Path.of(dir, "from-agent.txt")), wrote);
                perform("Homebody.ReviewChanges");
                String listed = answer("from-agent.txt", "Put Them Into This Project", 20000);
                check("the change is shown for review", listed != null && listed.contains("changed 1 file"), listed == null ? openDialogs() : listed);
                String done = answer("1 file(s) put into this project", "OK", 20000);
                check("taking it puts the file in your project", done != null && Files.exists(Path.of(dir, "from-agent.txt")), openDialogs());
            }

            // 5. A question from Homebody, answered here.
            // After a question nobody answered, Homebody stays quiet about that place for a while;
            // the launcher can name another one (PLACE=…) for a run that follows a failed one.
            String place = System.getProperty("homebody.selfcheck.place", "example.net");
            boolean ruled = false;
            for (JsonElement t : call("get_trust", HomebodyClient.args("appId", agent)).getAsJsonArray()) ruled |= HomebodyClient.text(t.getAsJsonObject(), "host").endsWith(place);
            if (ruled || (status.has("ask") && !status.get("ask").getAsBoolean())) {
                check("a question appears here and the answer counts", false, "not run: asking is off, or there is already a rule for " + place);
            } else {
                String[] code = {null};
                Thread curl = new Thread(() -> code[0] = sh(60, CLI, "run", agent, "--", "/usr/bin/curl", "-sS", "-m", "45", "-o", "/dev/null", "-w", "%{http_code}", "https://" + place + "/"));
                curl.start();
                Notification[] found = {null};
                boolean asked = until(() -> {
                    for (Notification n : NotificationsManager.getNotificationsManager().getNotificationsOfType(Notification.class, project)) {
                        if ("Homebody".equals(n.getGroupId()) && (n.getTitle() + " " + n.getContent()).contains(place)) found[0] = n;
                    }
                    return found[0] != null;
                }, 20000);
                check("a question appears here", asked, asked ? found[0].getTitle() + " " + found[0].getContent() : "no notification arrived");
                if (asked) {
                    List<AnAction> choices = found[0].getActions();
                    check("with its three answers", choices.size() == 3 && "Allow once".equals(choices.get(0).getTemplateText()), choices.stream().map(AnAction::getTemplateText).toList());
                    // Pressed the way the balloon's own link presses it (the editor supplies the notification).
                    ApplicationManager.getApplication().invokeLater(() -> Notification.fire(found[0], choices.get(0), null));
                }
                try {
                    curl.join(60000);
                } catch (InterruptedException ignored) {
                    // reported below
                }
                check("\"Allow once\" here lets that one connection through", "200".equals(code[0]), code[0]);
                boolean kept = false;
                for (JsonElement t : call("get_trust", HomebodyClient.args("appId", agent)).getAsJsonArray()) kept |= HomebodyClient.text(t.getAsJsonObject(), "host").endsWith(place);
                check("and leaves no lasting rule", !kept, "");
            }

            // 6. Block everything, and release.
            perform("Homebody.BlockEverything");
            boolean blocked = until(() -> status().get("lockdown").getAsBoolean(), 8000);
            check("\"Block Everything Now\" blocks everything", blocked, "");
            check("and the status bar says so", until(() -> "Homebody: everything blocked".equals(widgetText()), 8000), widgetText());
            perform("Homebody.Release");
            check("releasing lifts it", until(() -> !status().get("lockdown").getAsBoolean(), 8000) && until(() -> !"Homebody: everything blocked".equals(widgetText()), 8000), widgetText());

            List<String> left = openDialogs();
            check("no dialog is left open (an error would be one)", left.isEmpty(), left);
        } catch (Throwable e) {
            StringBuilder trace = new StringBuilder(String.valueOf(e));
            for (StackTraceElement s : e.getStackTrace()) if (s.getClassName().startsWith("app.homebody")) trace.append(" at ").append(s);
            check("the self-check ran to the end", false, trace);
        } finally {
            try {
                HomebodyClient.call("set_lockdown", HomebodyClient.args("on", false));
                HomebodyClient.call("app_stop", HomebodyClient.args("appId", agent));
                if (brought != null) HomebodyClient.call("project_remove", HomebodyClient.args("appId", agent, "name", HomebodyClient.text(brought, "name")));
            } catch (HomebodyClient.Refused e) {
                check("what the self-check did was undone", false, e.getMessage());
            }
            JsonObject report = new JsonObject();
            report.addProperty("editor", ApplicationInfo.getInstance().getFullApplicationName() + " (" + ApplicationInfo.getInstance().getBuild().asString() + ")");
            report.add("results", results);
            try {
                Files.writeString(out, new GsonBuilder().setPrettyPrinting().create().toJson(report));
            } catch (IOException ignored) {
                // the launcher reports that nothing was written
            }
        }
    }
}
