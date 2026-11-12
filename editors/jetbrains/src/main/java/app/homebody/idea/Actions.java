package app.homebody.idea;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.intellij.openapi.actionSystem.AnAction;
import com.intellij.openapi.actionSystem.AnActionEvent;
import com.intellij.openapi.application.ApplicationManager;
import com.intellij.openapi.project.Project;
import com.intellij.openapi.ui.Messages;
import com.intellij.util.concurrency.AppExecutorUtil;
import org.jetbrains.annotations.NotNull;

import java.util.ArrayList;
import java.util.List;

/** What the actions share: work off the interface thread, sentences shown on it. */
final class Actions {
    private Actions() {}

    interface Work {
        void run() throws HomebodyClient.Refused;
    }

    static void background(Project project, Work work) {
        AppExecutorUtil.getAppExecutorService().execute(() -> {
            try {
                work.run();
            } catch (HomebodyClient.Refused e) {
                say(project, e.getMessage(), true);
            }
        });
    }

    static void say(Project project, String message, boolean problem) {
        ApplicationManager.getApplication().invokeLater(() -> {
            if (problem) Messages.showErrorDialog(project, message, "Homebody");
            else Messages.showInfoMessage(project, message, "Homebody");
        });
    }

    /** Ask on the interface thread and wait for the answer. */
    static int choose(Project project, String message, String[] options) {
        int[] picked = {-1};
        ApplicationManager.getApplication().invokeAndWait(() ->
            picked[0] = Messages.showDialog(project, message, "Homebody", options, 0, Messages.getQuestionIcon()));
        return picked[0];
    }

    /** Agents that are installed, protected and ready. */
    static List<JsonObject> agents() throws HomebodyClient.Refused {
        JsonObject status = HomebodyClient.call("get_status", null).getAsJsonObject();
        List<JsonObject> out = new ArrayList<>();
        for (JsonElement a : status.getAsJsonArray("apps")) {
            JsonObject app = a.getAsJsonObject();
            if (HomebodyClient.text(app, "kind").equals("agent") && app.get("installed").getAsBoolean() && app.get("canStart").getAsBoolean()) out.add(app);
        }
        if (out.isEmpty()) throw new HomebodyClient.Refused("No protected agent is ready. Open Homebody to add one or to see why.");
        return out;
    }

    static JsonObject pickAgent(Project project) throws HomebodyClient.Refused {
        List<JsonObject> list = agents();
        if (list.size() == 1) return list.get(0);
        String[] names = list.stream().map(a -> HomebodyClient.text(a, "name")).toArray(String[]::new);
        int i = choose(project, "Which agent?", names);
        return i < 0 ? null : list.get(i);
    }

    /**
     * The project's folder by its real path. Homebody records a folder under the path it really
     * has, so one reached through a link (/tmp on a Mac, a linked Documents) is named the same way.
     */
    static String folder(Project project) {
        String base = project.getBasePath();
        if (base == null) return null;
        try {
            return java.nio.file.Path.of(base).toRealPath().toString();
        } catch (java.io.IOException | RuntimeException e) {
            return base;
        }
    }

    /** The agent's copy of this project, if the project has been brought in. */
    static JsonObject projectFor(JsonObject app, String dir) throws HomebodyClient.Refused {
        JsonArray list = HomebodyClient.call("get_projects", HomebodyClient.args("appId", HomebodyClient.text(app, "id"))).getAsJsonArray();
        for (JsonElement p : list) {
            if (HomebodyClient.text(p.getAsJsonObject(), "source").equals(dir)) return p.getAsJsonObject();
        }
        return null;
    }
}

/** Open an agent protected, working on a copy of this project. */
final class OpenAgentAction extends AnAction {
    @Override public void actionPerformed(@NotNull AnActionEvent e) {
        Project project = e.getProject();
        if (project == null || project.getBasePath() == null) return;
        String dir = Actions.folder(project);
        Actions.background(project, () -> {
            JsonObject app = Actions.pickAgent(project);
            if (app == null) return;
            String id = HomebodyClient.text(app, "id");
            JsonObject copy = Actions.projectFor(app, dir);
            if (copy == null) {
                int go = Actions.choose(project, HomebodyClient.text(app, "name") + " works on a copy of this project, so it cannot touch anything else of yours. You review its changes before they come back.", new String[]{"Bring This Project In", "Cancel"});
                if (go != 0) return;
                copy = HomebodyClient.call("project_import", HomebodyClient.args("appId", id, "path", dir)).getAsJsonObject();
            }
            HomebodyClient.call("app_open_terminal", HomebodyClient.args("appId", id, "project", HomebodyClient.text(copy, "name")));
        });
    }
}

/** List what the agent changed in its copy, and put the changes into the project when asked. */
final class ReviewChangesAction extends AnAction {
    @Override public void actionPerformed(@NotNull AnActionEvent e) {
        Project project = e.getProject();
        if (project == null || project.getBasePath() == null) return;
        String dir = Actions.folder(project);
        Actions.background(project, () -> {
            JsonObject status = HomebodyClient.call("get_status", null).getAsJsonObject();
            for (JsonElement a : status.getAsJsonArray("apps")) {
                JsonObject app = a.getAsJsonObject();
                if (!HomebodyClient.text(app, "kind").equals("agent")) continue;
                JsonObject copy = Actions.projectFor(app, dir);
                if (copy == null) continue;
                String id = HomebodyClient.text(app, "id"), name = HomebodyClient.text(copy, "name");
                JsonObject set = HomebodyClient.call("project_changes", HomebodyClient.args("appId", id, "name", name)).getAsJsonObject();
                int total = set.get("total").getAsInt();
                if (total == 0) {
                    Actions.say(project, HomebodyClient.text(app, "name") + " has changed nothing in its copy.", false);
                    return;
                }
                StringBuilder list = new StringBuilder(HomebodyClient.text(app, "name") + " changed " + total + " file(s):\n\n");
                int shown = 0;
                for (JsonElement c : set.getAsJsonArray("items")) {
                    JsonObject ch = c.getAsJsonObject();
                    if (shown++ == 25) { list.append("…\n"); break; }
                    list.append(HomebodyClient.text(ch, "kind")).append("  ").append(HomebodyClient.text(ch, "path"));
                    if (ch.has("conflict") && ch.get("conflict").getAsBoolean()) list.append("  (you changed this too)");
                    list.append('\n');
                }
                if (Actions.choose(project, list.toString(), new String[]{"Put Them Into This Project", "Not Now"}) != 0) return;
                JsonObject r = HomebodyClient.call("project_apply", HomebodyClient.args("appId", id, "name", name, "overwrite", false)).getAsJsonObject();
                int left = r.getAsJsonArray("skipped").size();
                Actions.say(project, r.get("applied").getAsInt() + " file(s) put into this project" + (left > 0 ? "; " + left + " left alone because you changed them too." : "."), false);
                return;
            }
            Actions.say(project, "This project has not been brought in for any agent.", false);
        });
    }
}

/** The last things your agents reached, and what was stopped. */
final class ShowActivityAction extends AnAction {
    @Override public void actionPerformed(@NotNull AnActionEvent e) {
        Project project = e.getProject();
        Actions.background(project, () -> {
            JsonArray rows = HomebodyClient.call("get_feed", HomebodyClient.args("limit", 25)).getAsJsonArray();
            StringBuilder out = new StringBuilder();
            for (JsonElement r : rows) {
                JsonObject ev = r.getAsJsonObject();
                String ts = HomebodyClient.text(ev, "ts");
                out.append(ts.length() >= 19 ? ts.substring(11, 19) : ts).append("  ")
                   .append(HomebodyClient.text(ev, "verb")).append("  ")
                   .append(HomebodyClient.text(ev, "appName")).append("  ")
                   .append(HomebodyClient.text(ev, "label")).append('\n');
            }
            Actions.say(project, out.length() == 0 ? "Nothing yet." : out.toString(), false);
        });
    }
}

final class BlockEverythingAction extends AnAction {
    @Override public void actionPerformed(@NotNull AnActionEvent e) {
        Actions.background(e.getProject(), () -> HomebodyClient.call("set_lockdown", HomebodyClient.args("on", true)));
    }
}

final class ReleaseAction extends AnAction {
    @Override public void actionPerformed(@NotNull AnActionEvent e) {
        Actions.background(e.getProject(), () -> HomebodyClient.call("set_lockdown", HomebodyClient.args("on", false)));
    }
}
