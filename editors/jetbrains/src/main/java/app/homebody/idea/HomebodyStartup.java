package app.homebody.idea;

import com.intellij.openapi.project.Project;
import com.intellij.openapi.startup.ProjectActivity;
import kotlin.Unit;
import kotlin.coroutines.Continuation;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

/**
 * Nothing to do at start beyond what the status bar entry does; kept so the plugin loads early.
 *
 * One exception, for testing: a build made with `-PselfCheck` carries an extra class that
 * exercises the plugin inside the running editor (src/selfcheck). It runs only when that class
 * is there and whoever started the editor asked for it. Released builds do not contain it.
 */
public final class HomebodyStartup implements ProjectActivity {
    @Override
    public @Nullable Object execute(@NotNull Project project, @NotNull Continuation<? super Unit> continuation) {
        String out = System.getProperty("homebody.selfcheck");
        if (out != null && !out.isEmpty()) {
            try {
                Class.forName("app.homebody.idea.SelfCheck").getMethod("start", Project.class, String.class).invoke(null, project, out);
            } catch (ReflectiveOperationException ignored) {
                // a released build: nothing to run
            }
        }
        return Unit.INSTANCE;
    }
}
