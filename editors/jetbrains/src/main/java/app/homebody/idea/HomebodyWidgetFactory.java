package app.homebody.idea;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.intellij.notification.Notification;
import com.intellij.notification.NotificationAction;
import com.intellij.notification.NotificationGroupManager;
import com.intellij.notification.NotificationType;
import com.intellij.openapi.actionSystem.ActionManager;
import com.intellij.openapi.actionSystem.ActionGroup;
import com.intellij.openapi.actionSystem.DataContext;
import com.intellij.openapi.actionSystem.impl.SimpleDataContext;
import com.intellij.openapi.actionSystem.CommonDataKeys;
import com.intellij.openapi.application.ApplicationManager;
import com.intellij.openapi.project.Project;
import com.intellij.openapi.ui.popup.JBPopupFactory;
import com.intellij.openapi.wm.StatusBar;
import com.intellij.openapi.wm.StatusBarWidget;
import com.intellij.openapi.wm.StatusBarWidgetFactory;
import com.intellij.ui.awt.RelativePoint;
import com.intellij.util.Consumer;
import com.intellij.util.concurrency.AppExecutorUtil;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

import java.awt.event.MouseEvent;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;

/** The status bar entry: what Homebody is doing, kept current, with the actions one click away. */
public final class HomebodyWidgetFactory implements StatusBarWidgetFactory {
    @Override public @NotNull String getId() { return "HomebodyStatus"; }
    @Override public @NotNull String getDisplayName() { return "Homebody"; }
    @Override public boolean isAvailable(@NotNull Project project) { return true; }
    @Override public @NotNull StatusBarWidget createWidget(@NotNull Project project) { return new Widget(project); }
    @Override public void disposeWidget(@NotNull StatusBarWidget widget) { widget.dispose(); }
    @Override public boolean canBeEnabledOn(@NotNull StatusBar statusBar) { return true; }

    static final class Widget implements StatusBarWidget, StatusBarWidget.TextPresentation {
        private final Project project;
        private StatusBar bar;
        private ScheduledFuture<?> tick;
        private volatile String text = "Homebody";
        private volatile String tip = "Homebody";
        private final Set<String> asked = new HashSet<>();

        Widget(Project project) { this.project = project; }

        @Override public @NotNull String ID() { return "HomebodyStatus"; }
        @Override public @Nullable WidgetPresentation getPresentation() { return this; }
        @Override public @NotNull String getText() { return text; }
        @Override public float getAlignment() { return 0.5f; }
        @Override public @Nullable String getTooltipText() { return tip; }

        @Override public void install(@NotNull StatusBar statusBar) {
            bar = statusBar;
            tick = AppExecutorUtil.getAppScheduledExecutorService().scheduleWithFixedDelay(this::refresh, 0, 3, TimeUnit.SECONDS);
        }

        @Override public void dispose() {
            if (tick != null) tick.cancel(false);
        }

        private void refresh() {
            JsonObject status = null;
            try {
                JsonElement v = HomebodyClient.call("get_status", null);
                status = v.isJsonObject() ? v.getAsJsonObject() : null;
            } catch (HomebodyClient.Refused ignored) {
                // not running: the text below says so
            }
            text = HomebodyClient.summary(status);
            tip = status == null ? "Open the Homebody app to protect your AI agents." : HomebodyClient.text(status, "sentence");
            if (status != null && status.has("pending")) {
                for (JsonElement q : status.getAsJsonArray("pending")) ask(q.getAsJsonObject());
            }
            StatusBar b = bar;
            if (b != null) ApplicationManager.getApplication().invokeLater(() -> b.updateWidget(ID()));
        }

        /** One of Homebody's questions, shown here once. Whoever answers first, here or in Homebody, wins. */
        private void ask(JsonObject q) {
            String id = HomebodyClient.text(q, "id");
            synchronized (asked) {
                if (id.isEmpty() || !asked.add(id)) return;
            }
            Notification n = NotificationGroupManager.getInstance().getNotificationGroup("Homebody")
                .createNotification(HomebodyClient.text(q, "title"), HomebodyClient.text(q, "body"), NotificationType.WARNING);
            for (String[] choice : new String[][]{{"Allow once", "once"}, {"Always allow", "always"}, {"Block", "block"}}) {
                n.addAction(NotificationAction.createSimpleExpiring(choice[0], () ->
                    AppExecutorUtil.getAppExecutorService().execute(() -> {
                        try {
                            HomebodyClient.call("decide", HomebodyClient.args("askId", id, "choice", choice[1]));
                        } catch (HomebodyClient.Refused ignored) {
                            // already answered elsewhere
                        }
                    })));
            }
            n.notify(project);
        }

        @Override public @Nullable Consumer<MouseEvent> getClickConsumer() {
            return event -> {
                ActionGroup group = (ActionGroup) ActionManager.getInstance().getAction("Homebody.Menu");
                DataContext context = SimpleDataContext.getSimpleContext(CommonDataKeys.PROJECT, project);
                JBPopupFactory.getInstance()
                    .createActionGroupPopup("Homebody", group, context, JBPopupFactory.ActionSelectionAid.SPEEDSEARCH, false)
                    .show(new RelativePoint(event));
            };
        }
    }
}
