package app.homebody.idea;

import com.intellij.openapi.project.Project;
import com.intellij.openapi.startup.ProjectActivity;
import kotlin.Unit;
import kotlin.coroutines.Continuation;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

/** Nothing to do at start beyond what the status bar entry does; kept so the plugin loads early. */
public final class HomebodyStartup implements ProjectActivity {
    @Override
    public @Nullable Object execute(@NotNull Project project, @NotNull Continuation<? super Unit> continuation) {
        return Unit.INSTANCE;
    }
}
